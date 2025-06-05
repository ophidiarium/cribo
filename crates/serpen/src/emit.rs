use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;

use crate::ast_rewriter::AstRewriter;
use crate::dependency_graph::ModuleNode;
use crate::resolver::{ImportType, ModuleResolver};
use crate::unused_imports_simple::UnusedImportAnalyzer;
use ruff_python_ast::{
    Alias, Arguments, Expr, ExprAttribute, ExprCall, ExprContext, ExprName, Identifier, ModModule,
    Stmt, StmtAssign, StmtImport, StmtImportFrom,
};
use ruff_python_codegen::{Generator, Stylist};
use ruff_python_parser;
use ruff_text_size::TextRange;

/// Type alias for import sets to reduce complexity
type ImportSets = (HashSet<String>, HashSet<String>);

/// Pre-parsed module data with AST for efficient processing
struct ParsedModuleData {
    ast: ModModule,
    unused_imports: HashSet<String>,
    first_party_imports: HashSet<String>,
}

/// Import strategy for how a module should be bundled
#[derive(Debug, Clone, PartialEq)]
enum ImportStrategy {
    /// Module imported via `import module` - needs namespace
    ModuleImport,
    /// Module imported via `from module import items` - needs direct inlining
    FromImport,
    /// Module not imported directly (dependency of other modules)
    Dependency,
}

pub struct CodeEmitter {
    resolver: ModuleResolver,
    _preserve_comments: bool,
    _preserve_type_hints: bool,
    /// Track which parent namespaces have already been created to avoid duplicates
    created_namespaces: HashSet<String>,
}

impl CodeEmitter {
    pub fn new(
        resolver: ModuleResolver,
        preserve_comments: bool,
        preserve_type_hints: bool,
    ) -> Self {
        Self {
            resolver,
            _preserve_comments: preserve_comments,
            _preserve_type_hints: preserve_type_hints,
            created_namespaces: HashSet::new(),
        }
    }

    /// Helper method to classify and add import to appropriate set
    fn classify_and_add_import(
        &self,
        import: &str,
        third_party_imports: &mut HashSet<String>,
        stdlib_imports: &mut HashSet<String>,
    ) {
        match self.resolver.classify_import(import) {
            ImportType::ThirdParty => {
                third_party_imports.insert(import.to_string());
            }
            ImportType::StandardLibrary => {
                stdlib_imports.insert(import.to_string());
            }
            ImportType::FirstParty => {
                // Will be inlined, so skip
            }
        }
    }

    /// Collect imports and categorize them by type
    fn collect_import_sets(&self, modules: &[&ModuleNode]) -> ImportSets {
        let mut third_party_imports = HashSet::new();
        let mut stdlib_imports = HashSet::new();

        for module in modules {
            for import in &module.imports {
                self.classify_and_add_import(import, &mut third_party_imports, &mut stdlib_imports);
            }
        }

        (third_party_imports, stdlib_imports)
    }

    /// Filter out imports that have alias assignments to avoid redundancy
    /// Returns the aliased imports that need to be added separately
    fn filter_aliased_imports(
        &self,
        third_party_imports: &mut HashSet<String>,
        stdlib_imports: &mut HashSet<String>,
        ast_rewriter: &AstRewriter,
    ) -> ImportSets {
        let import_aliases = ast_rewriter.import_aliases();

        // Collect aliased modules that need to be imported for alias assignments
        let mut aliased_third_party = HashSet::new();
        let mut aliased_stdlib = HashSet::new();

        for import_alias in import_aliases.values() {
            if import_alias.has_explicit_alias && !import_alias.is_from_import {
                let module_name = &import_alias.module_name;

                // Check if this module was in third_party or stdlib imports
                if third_party_imports.contains(module_name) {
                    aliased_third_party.insert(module_name.clone());
                    third_party_imports.remove(module_name);
                } else if stdlib_imports.contains(module_name) {
                    aliased_stdlib.insert(module_name.clone());
                    stdlib_imports.remove(module_name);
                }
            }
        }

        let filtered_count = aliased_third_party.len() + aliased_stdlib.len();
        if filtered_count > 0 {
            log::debug!(
                "Filtered {} aliased imports from preserved imports (will add separately)",
                filtered_count
            );
        }

        (aliased_third_party, aliased_stdlib)
    }

    /// Add aliased imports to the bundle separately (for alias assignments)
    fn add_aliased_imports_to_bundle(
        &self,
        bundle_ast: &mut ModModule,
        aliased_imports: ImportSets,
    ) {
        let (aliased_third_party, aliased_stdlib) = aliased_imports;

        if aliased_third_party.is_empty() && aliased_stdlib.is_empty() {
            return;
        }

        // Add comment for aliased imports section
        bundle_ast.body.push(self.create_comment_stmt(""));
        bundle_ast
            .body
            .push(self.create_comment_stmt("# Imports for alias assignments"));

        // Add stdlib imports first
        let mut sorted_stdlib: Vec<_> = aliased_stdlib.into_iter().collect();
        sorted_stdlib.sort();
        for import in sorted_stdlib {
            if let Some(stmt) = self.create_import_statement(&import) {
                bundle_ast.body.push(stmt);
            }
        }

        // Add third-party imports
        let mut sorted_third_party: Vec<_> = aliased_third_party.into_iter().collect();
        sorted_third_party.sort();
        for import in sorted_third_party {
            if let Some(stmt) = self.create_import_statement(&import) {
                bundle_ast.body.push(stmt);
            }
        }
    }

    /// Create an import statement using direct AST construction
    fn create_import_statement(&self, module_name: &str) -> Option<Stmt> {
        // Check if the module name is already a formatted import statement
        if module_name.starts_with("import ") || module_name.starts_with("from ") {
            // For pre-formatted imports, skip (these should be handled as comments separately)
            None
        } else if self.is_valid_module_name(module_name) {
            // Construct import statement directly using AST nodes
            Some(Stmt::Import(StmtImport {
                names: vec![Alias {
                    name: Identifier::new(module_name, TextRange::default()),
                    asname: None,
                    range: TextRange::default(),
                }],
                range: TextRange::default(),
            }))
        } else {
            // For unusual module names, skip
            None
        }
    }

    /// Generate bundled Python code from sorted modules using AST-based approach
    pub fn emit_bundle(&mut self, modules: &[&ModuleNode], entry_module: &str) -> Result<String> {
        // Create a main bundle AST that will contain everything
        let mut bundle_ast = ModModule {
            body: vec![
                // Add shebang and header comments
                self.create_comment_stmt("#!/usr/bin/env python3"),
                self.create_comment_stmt("# Generated by Serpen - Python Source Bundler"),
                self.create_comment_stmt("# https://github.com/tinovyatkin/serpen"),
                self.create_comment_stmt(""),
            ],
            range: Default::default(),
        };

        // Parse all modules once and store AST + metadata
        let mut all_unused_imports = HashSet::new();
        let mut parsed_modules_data = HashMap::new();

        for module in modules {
            let source = fs::read_to_string(&module.path)
                .with_context(|| format!("Failed to read module file: {:?}", module.path))?;

            // Parse into AST
            let ast = ruff_python_parser::parse_module(&source)
                .with_context(|| format!("Failed to parse module: {:?}", module.path))?;

            // Analyze unused imports
            let mut unused_analyzer = UnusedImportAnalyzer::new();
            let unused_imports = unused_analyzer.analyze_file(&source).unwrap_or_else(|err| {
                log::warn!(
                    "Failed to analyze unused imports in {:?}: {}",
                    module.path,
                    err
                );
                Vec::new()
            });

            let module_unused_names: HashSet<String> = unused_imports
                .iter()
                .map(|import| import.name.clone())
                .collect();

            // Collect first-party imports from AST
            let first_party_imports = self.collect_first_party_imports_from_ast(ast.syntax())?;

            // Store parsed data
            parsed_modules_data.insert(
                module.path.clone(),
                ParsedModuleData {
                    ast: ast.syntax().clone(),
                    unused_imports: module_unused_names.clone(),
                    first_party_imports,
                },
            );

            // Add to global unused set
            for import in unused_imports {
                all_unused_imports.insert(import.name);
            }
        }

        // Initialize AST rewriter for handling import aliases and name conflicts
        let python_version = self
            .resolver
            .config()
            .python_version()
            .context("Failed to parse target Python version")?;
        let mut ast_rewriter = AstRewriter::new(python_version);

        // Collect import aliases from the entry module before they are removed
        if let Some(entry_module_data) = modules
            .iter()
            .find(|m| m.name == entry_module)
            .and_then(|m| parsed_modules_data.get(&m.path))
        {
            ast_rewriter.collect_import_aliases(&entry_module_data.ast, entry_module);
        }

        // Pre-compute module import flags based on resolver information
        let module_flags = {
            let mut flags = std::collections::HashMap::new();
            for import_alias in ast_rewriter.import_aliases().values() {
                if import_alias.is_from_import {
                    let full_module_name = format!(
                        "{}.{}",
                        import_alias.module_name, import_alias.original_name
                    );
                    let is_module = self
                        .resolver
                        .resolve_module_path(&full_module_name)
                        .unwrap_or(None)
                        .is_some();
                    flags.insert(full_module_name, is_module);
                }
            }
            flags
        };

        // Update module import flags
        ast_rewriter.update_module_import_flags(|module_name| {
            module_flags.get(module_name).copied().unwrap_or(false)
        });

        // Analyze name conflicts across all modules
        let module_asts: Vec<(String, &ModModule)> = modules
            .iter()
            .filter_map(|m| {
                parsed_modules_data
                    .get(&m.path)
                    .map(|data| (m.name.clone(), &data.ast))
            })
            .collect();
        ast_rewriter.analyze_name_conflicts(&module_asts);

        log::info!("AST Rewriter Analysis:\n{}", ast_rewriter.get_debug_info());

        // Analyze import strategies for each module based on how they're imported by the entry module
        let import_strategies =
            self.analyze_import_strategies(modules, entry_module, &parsed_modules_data)?;

        // Collect and filter preserved imports
        let (mut third_party_imports, mut stdlib_imports) = self.collect_import_sets(modules);
        third_party_imports.retain(|import| !all_unused_imports.contains(import));
        stdlib_imports.retain(|import| !all_unused_imports.contains(import));

        // Filter out imports that have alias assignments to avoid redundancy
        let aliased_imports = self.filter_aliased_imports(
            &mut third_party_imports,
            &mut stdlib_imports,
            &ast_rewriter,
        );

        // Add preserved imports at the top
        self.add_preserved_imports_to_bundle(&mut bundle_ast, stdlib_imports, third_party_imports);

        // Add aliased imports separately (not in preserved imports section)
        self.add_aliased_imports_to_bundle(&mut bundle_ast, aliased_imports);

        // Process each module in dependency order using AST transformations
        for module in modules {
            if module.name == entry_module {
                continue;
            }

            // Add module header
            bundle_ast
                .body
                .push(self.create_module_header_comment(&module.name));

            let parsed_data = parsed_modules_data.get(&module.path).ok_or_else(|| {
                anyhow::anyhow!("Missing parsed data for module: {:?}", module.path)
            })?;

            let import_strategy = import_strategies
                .get(&module.name)
                .unwrap_or(&ImportStrategy::Dependency);

            // Process the module and get a transformed AST
            let module_ast = self.process_module_ast_to_ast(
                &module.name,
                parsed_data,
                import_strategy,
                &mut ast_rewriter,
            )?;

            // Extend the bundle AST with the module's statements
            bundle_ast.body.extend(module_ast.body);

            // Add an empty line between modules
            bundle_ast.body.push(self.create_comment_stmt(""));
        }

        // Add entry module last
        if let Some(entry_module_node) = modules.iter().find(|m| m.name == entry_module) {
            // Add entry module header
            bundle_ast
                .body
                .push(self.create_entry_module_header_comment(entry_module));

            let parsed_data = parsed_modules_data
                .get(&entry_module_node.path)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Missing parsed data for entry module: {:?}",
                        entry_module_node.path
                    )
                })?;

            // Process the entry module and get a transformed AST
            let mut entry_ast = self.process_module_ast_to_ast(
                &entry_module_node.name,
                parsed_data,
                &ImportStrategy::Dependency,
                &mut ast_rewriter,
            )?;

            // Add alias assignments at the beginning of the entry module
            let alias_assignments = ast_rewriter.generate_alias_assignments();
            let mut new_body = alias_assignments;
            new_body.extend(entry_ast.body);
            entry_ast.body = new_body;

            // Extend the bundle AST with the entry module's statements
            bundle_ast.body.extend(entry_ast.body);
        }

        // Generate the final Python code using ruff codegen with comment detection
        let empty_parsed = ruff_python_parser::parse_module("")?;
        let stylist = Stylist::from_tokens(empty_parsed.tokens(), "");

        let mut code_parts = Vec::new();

        for stmt in &bundle_ast.body {
            let generator = Generator::from(&stylist);
            let stmt_code = generator.stmt(stmt);

            // Detect and convert marked string literals to comments
            let converted_code = self.convert_comment_strings(stmt_code);

            // Skip pass statements that were placeholders
            if converted_code.trim() == "pass" {
                continue;
            }

            code_parts.push(converted_code);
        }

        // Normalize line endings for cross-platform consistency
        let bundled_code = code_parts.join("\n");
        Ok(self.normalize_line_endings(bundled_code))
    }

    /// Process a single module's AST to produce a transformed AST for bundling
    #[allow(clippy::too_many_arguments)]
    fn process_module_ast_to_ast(
        &mut self,
        module_name: &str,
        parsed_data: &ParsedModuleData,
        import_strategy: &ImportStrategy,
        ast_rewriter: &mut AstRewriter,
    ) -> Result<ModModule> {
        log::info!("Processing module AST '{}'", module_name);

        // Create a transformed AST by cloning the original
        let mut transformed_ast = parsed_data.ast.clone();

        // Apply AST rewriting for name conflict resolution FIRST
        ast_rewriter.rewrite_module_ast(module_name, &mut transformed_ast)?;

        // Apply import alias transformations BEFORE removing imports
        // This ensures that alias information is available when transforming expressions
        ast_rewriter.transform_module_ast(&mut transformed_ast)?;

        // Remove first-party imports and unused imports AFTER transformations
        self.remove_first_party_imports(&mut transformed_ast, &parsed_data.first_party_imports)?;
        self.remove_unused_imports(&mut transformed_ast, &parsed_data.unused_imports)?;

        // Apply bundling strategy based on how this module is imported
        match import_strategy {
            ImportStrategy::ModuleImport => {
                // For modules imported as "import module", create a module namespace using AST nodes
                let module_ast = ModModule {
                    body: self.create_module_namespace_ast(module_name, &transformed_ast)?,
                    range: Default::default(),
                };
                Ok(module_ast)
            }
            ImportStrategy::FromImport | ImportStrategy::Dependency => {
                // For modules imported as "from module import" or dependency modules,
                // return the transformed AST directly
                Ok(transformed_ast)
            }
        }
    }

    /// Create a module namespace using AST operations
    fn create_module_namespace_ast(
        &mut self,
        module_name: &str,
        module_ast: &ModModule,
    ) -> Result<Vec<Stmt>> {
        // Start with the types import and namespace creation
        let mut namespace_stmts = self.create_module_namespace_structure(module_name)?;

        // Convert the module AST to a string for the exec call
        // This is necessary because Python AST doesn't allow directly representing a module as an expression
        let module_code = {
            let empty_parsed = ruff_python_parser::parse_module("")?;
            let stylist = Stylist::from_tokens(empty_parsed.tokens(), "");

            // Generate code for each statement and combine them
            let mut code_parts = Vec::new();
            for stmt in &module_ast.body {
                let generator = Generator::from(&stylist);
                let stmt_code = generator.stmt(stmt);
                code_parts.push(stmt_code);
            }

            self.normalize_line_endings(code_parts.join("\n"))
        };

        // Add the exec call that will execute the module code in its namespace
        let exec_stmt = self.create_module_exec_statement(module_name, &module_code)?;
        namespace_stmts.push(exec_stmt);

        Ok(namespace_stmts)
    }

    /// Create module namespace structure as AST nodes
    fn create_module_namespace_structure(&mut self, module_name: &str) -> Result<Vec<Stmt>> {
        let mut statements = Vec::new();
        let mut needs_import_types = false;

        // 2. Create parent namespaces first for nested modules (e.g., greetings.greeting)
        if module_name.contains('.') {
            let parent_statements =
                self.create_parent_namespaces(module_name, &mut needs_import_types)?;
            statements.extend(parent_statements);
        }

        // Mark the main module namespace as created
        if !self.created_namespaces.contains(module_name) {
            self.created_namespaces.insert(module_name.to_string());
            needs_import_types = true;

            let module_assignment = self.create_main_module_assignment(module_name)?;
            statements.push(Stmt::Assign(module_assignment));
        }

        // 1. Add import types only if we created any namespaces
        if needs_import_types {
            let import_types = StmtImport {
                names: vec![Alias {
                    name: Identifier::new("types", TextRange::default()),
                    asname: None,
                    range: Default::default(),
                }],
                range: Default::default(),
            };
            statements.insert(0, Stmt::Import(import_types));
        }

        Ok(statements)
    }

    /// Create parent namespace statements for nested modules
    fn create_parent_namespaces(
        &mut self,
        module_name: &str,
        needs_import_types: &mut bool,
    ) -> Result<Vec<Stmt>> {
        let mut statements = Vec::new();
        let parts: Vec<&str> = module_name.split('.').collect();

        // Create each parent namespace level
        for i in 1..parts.len() {
            let parent_name = parts[..i].join(".");

            // Skip if this parent namespace was already created
            if self.created_namespaces.contains(&parent_name) {
                continue;
            }

            // Mark this namespace as created and create the assignment
            self.created_namespaces.insert(parent_name.clone());
            *needs_import_types = true;

            let parent_assignment =
                self.create_parent_namespace_assignment(&parts, i, &parent_name)?;
            statements.push(Stmt::Assign(parent_assignment));
        }

        Ok(statements)
    }

    /// Create assignment statement for parent namespace using direct AST construction
    fn create_parent_namespace_assignment(
        &self,
        parts: &[&str],
        i: usize,
        parent_name: &str,
    ) -> Result<StmtAssign> {
        // Create the target expression (left side of assignment)
        let target_expr = if i == 1 {
            // Simple case: greetings = ...
            Expr::Name(ExprName {
                id: Identifier::new(parts[0], TextRange::default()).into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })
        } else {
            // Complex case: greetings.submodule = ...
            let mut current_expr = Expr::Name(ExprName {
                id: Identifier::new(parts[0], TextRange::default()).into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            });

            // Build the attribute chain
            for &part in &parts[1..i] {
                current_expr = Expr::Attribute(ExprAttribute {
                    value: Box::new(current_expr),
                    attr: Identifier::new(part, TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                });
            }
            current_expr
        };

        // Create the value expression: types.ModuleType('parent_name')
        let value_expr = Expr::Call(ExprCall {
            func: Box::new(Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: Identifier::new("types", TextRange::default()).into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("ModuleType", TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: Arguments {
                args: vec![Expr::StringLiteral(ruff_python_ast::ExprStringLiteral {
                    value: ruff_python_ast::StringLiteralValue::single(
                        ruff_python_ast::StringLiteral {
                            range: TextRange::default(),
                            value: parent_name.to_string().into_boxed_str(),
                            flags: ruff_python_ast::StringLiteralFlags::empty(),
                        },
                    ),
                    range: TextRange::default(),
                })]
                .into(),
                keywords: vec![].into(),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        Ok(StmtAssign {
            targets: vec![target_expr],
            value: Box::new(value_expr),
            range: TextRange::default(),
        })
    }

    /// Create assignment statement for main module namespace using direct AST construction
    fn create_main_module_assignment(&self, module_name: &str) -> Result<StmtAssign> {
        // Create the target expression: module_name = ...
        let target_expr = if module_name.contains('.') {
            // Handle dotted module names like "greetings.greeting"
            let parts: Vec<&str> = module_name.split('.').collect();
            let mut current_expr = Expr::Name(ExprName {
                id: Identifier::new(parts[0], TextRange::default()).into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            });

            // Build the attribute chain, with the last part having Store context
            for (idx, &part) in parts[1..].iter().enumerate() {
                let is_last = idx == parts.len() - 2;
                current_expr = Expr::Attribute(ExprAttribute {
                    value: Box::new(current_expr),
                    attr: Identifier::new(part, TextRange::default()),
                    ctx: if is_last {
                        ExprContext::Store
                    } else {
                        ExprContext::Load
                    },
                    range: TextRange::default(),
                });
            }
            current_expr
        } else {
            // Simple module name
            Expr::Name(ExprName {
                id: Identifier::new(module_name, TextRange::default()).into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })
        };

        // Create the value expression: types.ModuleType('module_name')
        let value_expr = Expr::Call(ExprCall {
            func: Box::new(Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: Identifier::new("types", TextRange::default()).into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("ModuleType", TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: Arguments {
                args: vec![Expr::StringLiteral(ruff_python_ast::ExprStringLiteral {
                    value: ruff_python_ast::StringLiteralValue::single(
                        ruff_python_ast::StringLiteral {
                            range: TextRange::default(),
                            value: module_name.to_string().into_boxed_str(),
                            flags: ruff_python_ast::StringLiteralFlags::empty(),
                        },
                    ),
                    range: TextRange::default(),
                })]
                .into(),
                keywords: vec![].into(),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        Ok(StmtAssign {
            targets: vec![target_expr],
            value: Box::new(value_expr),
            range: TextRange::default(),
        })
    }

    /// Create an AST for the exec statement that executes module code in its namespace
    fn create_module_exec_statement(&self, module_name: &str, module_code: &str) -> Result<Stmt> {
        // Create the exec call: exec(module_code, module_name.__dict__)
        let exec_call = Expr::Call(ExprCall {
            func: Box::new(Expr::Name(ExprName {
                id: Identifier::new("exec", TextRange::default()).into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: Arguments {
                args: vec![
                    // First argument: the module code as a string literal
                    Expr::StringLiteral(ruff_python_ast::ExprStringLiteral {
                        value: ruff_python_ast::StringLiteralValue::single(
                            ruff_python_ast::StringLiteral {
                                range: TextRange::default(),
                                value: module_code.to_string().into_boxed_str(),
                                flags: ruff_python_ast::StringLiteralFlags::empty(),
                            },
                        ),
                        range: TextRange::default(),
                    }),
                    // Second argument: module_name.__dict__
                    Expr::Attribute(ExprAttribute {
                        value: Box::new(if module_name.contains('.') {
                            // Handle dotted module names
                            let parts: Vec<&str> = module_name.split('.').collect();
                            let mut current_expr = Expr::Name(ExprName {
                                id: Identifier::new(parts[0], TextRange::default()).into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            });

                            for &part in &parts[1..] {
                                current_expr = Expr::Attribute(ExprAttribute {
                                    value: Box::new(current_expr),
                                    attr: Identifier::new(part, TextRange::default()),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                });
                            }
                            current_expr
                        } else {
                            // Simple module name
                            Expr::Name(ExprName {
                                id: Identifier::new(module_name, TextRange::default()).into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })
                        }),
                        attr: Identifier::new("__dict__", TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    }),
                ]
                .into(),
                keywords: vec![].into(),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        Ok(Stmt::Expr(ruff_python_ast::StmtExpr {
            value: Box::new(exec_call),
            range: TextRange::default(),
        }))
    }

    /// Analyze how each module is imported by the entry module to determine bundling strategy
    fn analyze_import_strategies(
        &self,
        modules: &[&ModuleNode],
        entry_module: &str,
        parsed_modules_data: &HashMap<std::path::PathBuf, ParsedModuleData>,
    ) -> Result<HashMap<String, ImportStrategy>> {
        let mut strategies = HashMap::new();

        // Find the entry module data
        let entry_module_node = modules
            .iter()
            .find(|m| m.name == entry_module)
            .ok_or_else(|| anyhow::anyhow!("Entry module not found: {}", entry_module))?;

        let entry_parsed_data = parsed_modules_data
            .get(&entry_module_node.path)
            .ok_or_else(|| anyhow::anyhow!("Entry module data not found"))?;

        // Analyze import statements in the entry module
        for stmt in &entry_parsed_data.ast.body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    self.process_import_strategies(import_stmt, &mut strategies);
                }
                Stmt::ImportFrom(import_from_stmt) => {
                    self.process_import_from_strategies(import_from_stmt, &mut strategies);
                }
                _ => {}
            }
        }

        Ok(strategies)
    }

    /// Process import statements for strategy analysis
    fn process_import_strategies(
        &self,
        import_stmt: &StmtImport,
        strategies: &mut HashMap<String, ImportStrategy>,
    ) {
        // `import module` - needs namespace
        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();
            if self.is_first_party_module(module_name) {
                strategies.insert(module_name.to_string(), ImportStrategy::ModuleImport);
            }
        }
    }

    /// Process import-from statements for strategy analysis
    fn process_import_from_strategies(
        &self,
        import_from_stmt: &StmtImportFrom,
        strategies: &mut HashMap<String, ImportStrategy>,
    ) {
        // `from module import ...` - needs direct inlining
        if let Some(module) = &import_from_stmt.module {
            let module_name = module.as_str();
            if self.is_first_party_module(module_name) {
                strategies.insert(module_name.to_string(), ImportStrategy::FromImport);

                // Check if any of the imported names are actually modules
                for alias in &import_from_stmt.names {
                    let imported_name = alias.name.as_str();
                    let full_module_name = format!("{}.{}", module_name, imported_name);

                    // If the imported name resolves to a module, it needs module import strategy
                    if self.is_first_party_module(&full_module_name) {
                        strategies.insert(full_module_name, ImportStrategy::ModuleImport);
                    }
                }
            }
        }
    }

    /// Check if a module is a first-party module
    fn is_first_party_module(&self, module_name: &str) -> bool {
        matches!(
            self.resolver.classify_import(module_name),
            ImportType::FirstParty
        )
    }

    /// Remove first-party imports from AST (they will be inlined)
    fn remove_first_party_imports(
        &self,
        module: &mut ModModule,
        first_party_imports: &HashSet<String>,
    ) -> Result<()> {
        log::info!("Removing first-party imports: {:?}", first_party_imports);
        module.body = self.filter_import_statements(&module.body, |import_name| {
            let is_first_party = first_party_imports.contains(import_name);
            let classification = self.resolver.classify_import(import_name);
            let keep = !is_first_party
                || matches!(
                    classification,
                    ImportType::StandardLibrary | ImportType::ThirdParty
                );
            log::info!(
                "Import '{}': first_party={}, classification={:?}, keep={}",
                import_name,
                is_first_party,
                classification,
                keep
            );
            keep
        })?;
        Ok(())
    }

    /// Remove unused imports from AST
    fn remove_unused_imports(
        &self,
        module: &mut ModModule,
        unused_imports: &HashSet<String>,
    ) -> Result<()> {
        module.body = self.filter_import_statements(&module.body, |import_name| {
            !unused_imports.contains(import_name)
        })?;
        Ok(())
    }

    /// Filter import statements based on a predicate
    fn filter_import_statements<F>(
        &self,
        statements: &[Stmt],
        keep_predicate: F,
    ) -> Result<Vec<Stmt>>
    where
        F: Fn(&str) -> bool,
    {
        let mut filtered_statements = Vec::new();

        for stmt in statements {
            match stmt {
                Stmt::Import(import_stmt) => {
                    self.process_import_statement(
                        import_stmt,
                        &keep_predicate,
                        &mut filtered_statements,
                    );
                }
                Stmt::ImportFrom(import_from_stmt) => {
                    self.process_import_from_statement(
                        import_from_stmt,
                        &keep_predicate,
                        &mut filtered_statements,
                    );
                }
                _ => {
                    // Keep all non-import statements as-is
                    filtered_statements.push(stmt.clone());
                }
            }
        }

        Ok(filtered_statements)
    }

    /// Process a single import statement
    fn process_import_statement<F>(
        &self,
        import_stmt: &StmtImport,
        keep_predicate: &F,
        filtered_statements: &mut Vec<Stmt>,
    ) where
        F: Fn(&str) -> bool,
    {
        let filtered_aliases = self.filter_import_aliases(&import_stmt.names, keep_predicate);

        if !filtered_aliases.is_empty() {
            let mut new_import = import_stmt.clone();
            new_import.names = filtered_aliases;
            filtered_statements.push(Stmt::Import(new_import));
        }
    }

    /// Process a single import-from statement
    fn process_import_from_statement<F>(
        &self,
        import_from_stmt: &StmtImportFrom,
        keep_predicate: &F,
        filtered_statements: &mut Vec<Stmt>,
    ) where
        F: Fn(&str) -> bool,
    {
        let should_keep = self.should_keep_import_from_module(import_from_stmt, keep_predicate);

        if should_keep {
            let filtered_aliases =
                self.filter_import_from_aliases(&import_from_stmt.names, keep_predicate);

            if !filtered_aliases.is_empty() {
                let mut new_import = import_from_stmt.clone();
                new_import.names = filtered_aliases;
                filtered_statements.push(Stmt::ImportFrom(new_import));
            }
        }
    }

    /// Filter aliases for regular import statements
    fn filter_import_aliases<F>(&self, aliases: &[Alias], keep_predicate: &F) -> Vec<Alias>
    where
        F: Fn(&str) -> bool,
    {
        aliases
            .iter()
            .filter(|alias| {
                let import_name = alias.name.as_str();
                keep_predicate(import_name)
            })
            .cloned()
            .collect()
    }

    /// Filter aliases for import-from statements
    fn filter_import_from_aliases<F>(&self, aliases: &[Alias], keep_predicate: &F) -> Vec<Alias>
    where
        F: Fn(&str) -> bool,
    {
        aliases
            .iter()
            .filter(|alias| {
                let local_name = alias
                    .asname
                    .as_ref()
                    .map(|n| n.as_str())
                    .unwrap_or_else(|| alias.name.as_str());
                keep_predicate(local_name)
            })
            .cloned()
            .collect()
    }

    /// Determine if an import-from statement's module should be kept
    fn should_keep_import_from_module<F>(
        &self,
        import_from_stmt: &StmtImportFrom,
        keep_predicate: &F,
    ) -> bool
    where
        F: Fn(&str) -> bool,
    {
        if let Some(module) = &import_from_stmt.module {
            keep_predicate(module.as_str())
        } else {
            // Relative import - keep for now (could be refined later)
            true
        }
    }

    /// Collect first-party imports from AST instead of re-parsing source
    fn collect_first_party_imports_from_ast(&self, module: &ModModule) -> Result<HashSet<String>> {
        let mut first_party_imports = HashSet::new();

        for stmt in &module.body {
            self.collect_first_party_from_statement(stmt, &mut first_party_imports);
        }

        log::debug!("Collected first-party imports: {:?}", first_party_imports);
        Ok(first_party_imports)
    }

    /// Extract first-party imports from an AST statement
    fn collect_first_party_from_statement(&self, stmt: &Stmt, imports: &mut HashSet<String>) {
        match stmt {
            Stmt::Import(import_stmt) => {
                self.collect_first_party_from_import(import_stmt, imports);
            }
            Stmt::ImportFrom(import_from_stmt) => {
                self.collect_first_party_from_import_from(import_from_stmt, imports);
            }
            _ => {}
        }
    }

    /// Helper to collect first-party imports from "from ... import" statements
    fn collect_first_party_from_import_from(
        &self,
        import_from_stmt: &StmtImportFrom,
        imports: &mut HashSet<String>,
    ) {
        // Handle relative imports (e.g., `from . import x`) as first-party
        if import_from_stmt.module.is_none() {
            if import_from_stmt.level > 0 {
                // Insert empty string marker for relative import
                imports.insert(String::new());
            }
            return;
        }
        let module = import_from_stmt.module.as_ref().unwrap();

        let module_name = module.as_str();
        if matches!(
            self.resolver.classify_import(module_name),
            ImportType::FirstParty
        ) {
            imports.insert(module_name.to_string());
        }
    }

    /// Helper to collect first-party imports from regular import statements
    fn collect_first_party_from_import(
        &self,
        import_stmt: &StmtImport,
        imports: &mut HashSet<String>,
    ) {
        for alias in &import_stmt.names {
            let import_name = alias.name.as_str();
            if matches!(
                self.resolver.classify_import(import_name),
                ImportType::FirstParty
            ) {
                imports.insert(import_name.to_string());
            }
        }
    }

    /// Check if a module name is valid (alphanumeric, underscores, dots only)
    fn is_valid_module_name(&self, module_name: &str) -> bool {
        !module_name.is_empty()
            && !module_name.contains(' ')
            && module_name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    }

    /// Generate requirements.txt content from third-party imports
    pub fn generate_requirements(&mut self, modules: &[&ModuleNode]) -> Result<String> {
        let mut third_party_imports = HashSet::new();

        for module in modules {
            self.collect_third_party_imports_from_module(module, &mut third_party_imports);
        }

        let mut requirements: Vec<String> = third_party_imports.into_iter().collect();
        requirements.sort();

        Ok(requirements.join("\n"))
    }

    /// Collect third-party imports from a single module
    fn collect_third_party_imports_from_module(
        &mut self,
        module: &ModuleNode,
        third_party_imports: &mut HashSet<String>,
    ) {
        for import in &module.imports {
            if let ImportType::ThirdParty = self.resolver.classify_import(import) {
                // Extract top-level package name
                let package_name = import.split('.').next().unwrap_or(import);
                third_party_imports.insert(package_name.to_string());
            }
        }
    }

    /// Create a comment as a string literal expression with a unique marker
    fn create_comment_stmt(&self, comment: &str) -> Stmt {
        // Use a unique marker that won't appear in normal Python code
        let comment_content = format!("__SERPEN_COMMENT_MARKER__{}", comment);

        // Create a string literal expression statement
        Stmt::Expr(ruff_python_ast::StmtExpr {
            value: Box::new(Expr::StringLiteral(ruff_python_ast::ExprStringLiteral {
                value: ruff_python_ast::StringLiteralValue::single(
                    ruff_python_ast::StringLiteral {
                        range: TextRange::default(),
                        value: comment_content.into_boxed_str(),
                        flags: ruff_python_ast::StringLiteralFlags::empty(),
                    },
                ),
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Convert marked string literals back to proper comments
    fn convert_comment_strings(&self, code: String) -> String {
        let trimmed = code.trim();

        // Check for our specific marker pattern - the generated code will be in quotes
        if trimmed.starts_with("'__SERPEN_COMMENT_MARKER__") && trimmed.ends_with("'") {
            // Extract the comment content
            let start_idx = "'__SERPEN_COMMENT_MARKER__".len();
            let content = &trimmed[start_idx..trimmed.len() - 1]; // Skip marker and quotes
            content.to_string()
        } else if trimmed.starts_with("\"__SERPEN_COMMENT_MARKER__") && trimmed.ends_with("\"") {
            // Extract the comment content (double quotes)
            let start_idx = "\"__SERPEN_COMMENT_MARKER__".len();
            let content = &trimmed[start_idx..trimmed.len() - 1]; // Skip marker and quotes
            content.to_string()
        } else {
            // Not a marked comment, return as-is
            code
        }
    }

    /// Create a module header comment
    fn create_module_header_comment(&self, module_name: &str) -> Stmt {
        self.create_comment_stmt(&format!("# ─ Module: {} ─", module_name))
    }

    /// Create entry module header comment
    fn create_entry_module_header_comment(&self, module_name: &str) -> Stmt {
        self.create_comment_stmt(&format!("# ─ Entry Module: {} ─", module_name))
    }

    /// Create preserved imports header comment
    fn create_preserved_imports_header(&self) -> Stmt {
        self.create_comment_stmt("# Preserved imports")
    }

    /// Add preserved imports to the bundle AST
    fn add_preserved_imports_to_bundle(
        &self,
        bundle_ast: &mut ModModule,
        stdlib_imports: HashSet<String>,
        third_party_imports: HashSet<String>,
    ) {
        if stdlib_imports.is_empty() && third_party_imports.is_empty() {
            return;
        }

        // Add preserved imports header comment
        bundle_ast.body.push(self.create_preserved_imports_header());

        // Standard library imports first
        let mut sorted_stdlib: Vec<_> = stdlib_imports.into_iter().collect();
        sorted_stdlib.sort();
        for import in &sorted_stdlib {
            // Create an import statement for standard library imports
            let Some(import_stmt) = self.create_import_statement(import) else {
                continue;
            };
            bundle_ast.body.push(import_stmt);
        }

        // Add an empty line comment between stdlib and third-party imports if both are present
        if !sorted_stdlib.is_empty() && !third_party_imports.is_empty() {
            bundle_ast.body.push(self.create_comment_stmt(""));
        }

        // Third-party imports
        let mut sorted_third_party: Vec<_> = third_party_imports.into_iter().collect();
        sorted_third_party.sort();
        for import in sorted_third_party {
            // Create an import statement for third-party imports
            let Some(import_stmt) = self.create_import_statement(&import) else {
                continue;
            };
            bundle_ast.body.push(import_stmt);
        }

        // Add an empty line comment after imports
        bundle_ast.body.push(self.create_comment_stmt(""));
    }

    /// Normalize line endings to LF (\n) for cross-platform consistency
    /// This ensures reproducible builds regardless of the platform where bundling occurs
    fn normalize_line_endings(&self, content: String) -> String {
        // Replace Windows CRLF (\r\n) and Mac CR (\r) with Unix LF (\n)
        content.replace("\r\n", "\n").replace('\r', "\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    fn create_test_emitter() -> CodeEmitter {
        let config = Config::default();
        let resolver = ModuleResolver::new(config).unwrap();
        CodeEmitter::new(resolver, false, false)
    }

    #[test]
    fn test_filter_import_statements_basic_functionality() {
        let emitter = create_test_emitter();

        // This is just a simple test to verify the AST-based approach works
        let source = "import os\nimport sys\n";
        if let Ok(module) = ruff_python_parser::parse_module(source) {
            let keep_predicate = |_module: &str| true; // Keep all imports for this test
            let filtered = emitter
                .filter_import_statements(&module.syntax().body, keep_predicate)
                .unwrap();

            // Should keep both import statements
            assert_eq!(filtered.len(), 2);
        }
    }
}
