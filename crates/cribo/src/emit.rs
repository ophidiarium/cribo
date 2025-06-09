use anyhow::{Context, Result};
use indexmap::{IndexMap, IndexSet};
use once_cell::sync::Lazy;
use regex::Regex;
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
type ImportSets = (IndexSet<String>, IndexSet<String>);

/// Cached regex for detecting bundled variable references like "__module_name_variable"
static BUNDLED_VAR_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"__[a-zA-Z_][a-zA-Z0-9_]*_[a-zA-Z_][a-zA-Z0-9_]*")
        .expect("Invalid regex pattern for bundled variable detection")
});

/// Pre-parsed module data with AST for efficient processing
struct ParsedModuleData {
    ast: ModModule,
    unused_imports: IndexSet<String>,
    first_party_imports: IndexSet<String>,
}

/// Import strategy for how a module should be bundled
#[derive(Debug, Clone, PartialEq)]
pub enum ImportStrategy {
    /// Module imported via `import module` - needs namespace
    ModuleImport,
    /// Module imported via `from module import items` - needs direct inlining
    FromImport,
    /// Module not imported directly (dependency of other modules)
    Dependency,
}

/// Parameters for creating module namespace AST
struct ModuleNamespaceParams<'a> {
    module_name: &'a str,
    module_ast: &'a ModModule,
    ast_rewriter: &'a AstRewriter,
    import_strategy: &'a ImportStrategy,
}

/// Parameters for creating module exec statement
struct ModuleExecParams<'a> {
    module_name: &'a str,
    module_code: &'a str,
    include_globals: bool,
    import_strategy: &'a ImportStrategy,
}

/// Parameters for transforming imports in init.py files
struct TransformImportsParams<'a> {
    module_name: &'a str,
    module_ast: &'a mut ModModule,
    import_strategies: &'a IndexMap<String, ImportStrategy>,
}

pub struct CodeEmitter {
    resolver: ModuleResolver,
    _preserve_comments: bool,
    _preserve_type_hints: bool,
    /// Track which parent namespaces have already been created to avoid duplicates
    created_namespaces: IndexSet<String>,
    /// Track which modules are from __init__.py files
    init_modules: IndexSet<String>,
    /// Track bundled variable names for each module
    bundled_variables: IndexMap<String, IndexMap<String, String>>,
    /// Track future imports that need to be hoisted to the top
    future_imports: IndexSet<String>,
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
            created_namespaces: IndexSet::new(),
            init_modules: IndexSet::new(),
            bundled_variables: IndexMap::new(),
            future_imports: IndexSet::new(),
        }
    }

    /// Helper function to create an ExprName node
    fn create_name_expr(name: &str, ctx: ExprContext) -> Expr {
        Expr::Name(ExprName {
            id: Identifier::new(name, TextRange::default()).into(),
            ctx,
            range: TextRange::default(),
        })
    }

    /// Helper function to create an ExprAttribute node
    fn create_attribute_expr(value: Expr, attr: &str, ctx: ExprContext) -> Expr {
        Expr::Attribute(ExprAttribute {
            value: Box::new(value),
            attr: Identifier::new(attr, TextRange::default()),
            ctx,
            range: TextRange::default(),
        })
    }

    /// Helper function to create a string literal expression
    fn create_string_literal(value: &str) -> Expr {
        Expr::StringLiteral(ruff_python_ast::ExprStringLiteral {
            value: ruff_python_ast::StringLiteralValue::single(ruff_python_ast::StringLiteral {
                range: TextRange::default(),
                value: value.to_string().into_boxed_str(),
                flags: ruff_python_ast::StringLiteralFlags::empty(),
            }),
            range: TextRange::default(),
        })
    }

    /// Helper function to create a getattr() call expression
    fn create_getattr_call(obj_expr: Expr, attr_name: &str) -> Expr {
        Expr::Call(ExprCall {
            func: Box::new(Self::create_name_expr("getattr", ExprContext::Load)),
            arguments: Arguments {
                args: vec![obj_expr, Self::create_string_literal(attr_name)].into(),
                keywords: vec![].into(),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        })
    }

    /// Helper function to build a dotted module expression from parts
    fn create_dotted_module_expr(parts: &[&str], ctx: ExprContext) -> Expr {
        let mut current_expr = Self::create_name_expr(parts[0], ExprContext::Load);

        let len = parts.len();
        for (idx, &part) in parts[1..].iter().enumerate() {
            let attr_ctx = if idx + 2 == len {
                ctx
            } else {
                ExprContext::Load
            };
            current_expr = Self::create_attribute_expr(current_expr, part, attr_ctx);
        }
        current_expr
    }

    /// Helper method to classify and add import to appropriate set
    fn classify_and_add_import(
        &self,
        import: &str,
        third_party_imports: &mut IndexSet<String>,
        stdlib_imports: &mut IndexSet<String>,
    ) {
        // Skip __future__ imports as they are handled separately
        if import == "__future__" {
            return;
        }

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
        let mut third_party_imports = IndexSet::new();
        let mut stdlib_imports = IndexSet::new();

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
        third_party_imports: &mut IndexSet<String>,
        stdlib_imports: &mut IndexSet<String>,
        ast_rewriter: &AstRewriter,
    ) -> ImportSets {
        let import_aliases = ast_rewriter.import_aliases();

        // Collect aliased modules that need to be imported for alias assignments
        let mut aliased_third_party = IndexSet::new();
        let mut aliased_stdlib = IndexSet::new();

        // Sort import aliases by key for deterministic processing order
        let mut sorted_aliases: Vec<_> = import_aliases.iter().collect();
        sorted_aliases.sort_by_key(|(key, _)| *key);

        for (_, import_alias) in sorted_aliases {
            if !import_alias.has_explicit_alias || import_alias.is_from_import {
                continue;
            }
            let module_name = &import_alias.module_name;
            // Check if this module was in third_party or stdlib imports
            if third_party_imports.contains(module_name) {
                aliased_third_party.insert(module_name.clone());
                third_party_imports.shift_remove(module_name);
            } else if stdlib_imports.contains(module_name) {
                aliased_stdlib.insert(module_name.clone());
                stdlib_imports.shift_remove(module_name);
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

    /// Add future imports to the bundle at the very top
    fn add_future_imports_to_bundle(&self, bundle_ast: &mut ModModule) {
        if self.future_imports.is_empty() {
            return;
        }

        // Sort future imports for deterministic output
        let mut sorted_features: Vec<_> = self.future_imports.iter().collect();
        sorted_features.sort();

        // Create a single from __future__ import statement with all features
        let future_import = Stmt::ImportFrom(StmtImportFrom {
            module: Some(Identifier::new("__future__", TextRange::default())),
            names: sorted_features
                .into_iter()
                .map(|feature| Alias {
                    name: Identifier::new(feature.clone(), TextRange::default()),
                    asname: None,
                    range: TextRange::default(),
                })
                .collect(),
            level: 0,
            range: TextRange::default(),
        });

        bundle_ast.body.push(future_import);
        bundle_ast.body.push(self.create_comment_stmt("")); // Add blank line after future imports
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
                self.create_comment_stmt("# Generated by Cribo - Python Source Bundler"),
                self.create_comment_stmt("# https://github.com/ophidiarium/cribo"),
                self.create_comment_stmt(""),
            ],
            range: Default::default(),
        };

        // Parse all modules once and store AST + metadata
        let mut all_unused_imports = IndexSet::new();
        let mut parsed_modules_data = IndexMap::new();

        for module in modules {
            // Check if this module is from an __init__.py file
            if module.path.file_name() == Some(std::ffi::OsStr::new("__init__.py")) {
                self.init_modules.insert(module.name.clone());
            }

            let source = fs::read_to_string(&module.path)
                .with_context(|| format!("Failed to read module file: {:?}", module.path))?;
            let source = crate::util::normalize_line_endings(source);

            // Parse into AST
            let ast = ruff_python_parser::parse_module(&source)
                .with_context(|| format!("Failed to parse module: {:?}", module.path))?;

            // Analyze unused imports with __init__.py awareness
            let is_init_py = module.path.file_name() == Some(std::ffi::OsStr::new("__init__.py"));
            let mut unused_analyzer = UnusedImportAnalyzer::new();
            let unused_imports = unused_analyzer
                .analyze_file_with_init_check(&source, is_init_py)
                .unwrap_or_else(|err| {
                    log::warn!(
                        "Failed to analyze unused imports in {:?}: {}",
                        module.path,
                        err
                    );
                    Vec::new()
                });

            let module_unused_names: IndexSet<String> = unused_imports
                .iter()
                .map(|import| import.name.clone())
                .collect();

            // Collect first-party imports from AST
            let first_party_imports = self.collect_first_party_imports_from_ast(ast.syntax())?;

            // Collect future imports
            self.collect_future_imports_from_ast(ast.syntax());

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

        // Set the init_modules set so the AST rewriter can accurately identify package interfaces
        ast_rewriter.set_init_modules(&self.init_modules);

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
            let mut flags = IndexMap::new();

            // Sort import aliases by key for deterministic processing order
            let mut sorted_from_aliases: Vec<_> = ast_rewriter
                .import_aliases()
                .iter()
                .filter(|(_, a)| a.is_from_import)
                .collect();
            sorted_from_aliases.sort_by_key(|(key, _)| *key);

            for (_, import_alias) in sorted_from_aliases {
                let full_module_name = format!(
                    "{}.{}",
                    import_alias.module_name, import_alias.original_name
                );

                // Check if this is actually a module import by verifying the import pattern:
                // - from package import module (where module.py exists) -> module import
                // - from package.module import variable -> variable import
                let is_module = self.is_true_module_import(import_alias, &full_module_name);
                flags.insert(full_module_name, is_module);
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

        // Pass import strategies to AST rewriter so it can resolve references correctly
        ast_rewriter.set_import_strategies(&import_strategies);

        // Pre-collect bundled variables from all modules to ensure complete mapping
        // This must be done before any transformations to ensure the bundled_modules mapping is complete
        for module in modules {
            let parsed_data = parsed_modules_data.get(&module.path).ok_or_else(|| {
                anyhow::anyhow!("Missing parsed data for module: {:?}", module.path)
            })?;

            // Create a temporary AST for symbol collection
            let mut temp_ast = parsed_data.ast.clone();
            ast_rewriter.rewrite_module_ast(&module.name, &mut temp_ast)?;

            // Collect bundled variables from the rewriter for this module
            self.collect_bundled_variables_from_rewriter(&module.name, &ast_rewriter);
        }

        // Collect and filter preserved imports
        let (mut third_party_imports, mut stdlib_imports) = self.collect_import_sets(modules);
        third_party_imports.retain(|import| !all_unused_imports.contains(import));
        stdlib_imports.retain(|import| !all_unused_imports.contains(import));

        // Filter out redundant general imports (e.g., "import typing" when "from typing import Dict" exists)
        third_party_imports =
            self.filter_redundant_imports_from_modules(third_party_imports, &parsed_modules_data);
        stdlib_imports =
            self.filter_redundant_imports_from_modules(stdlib_imports, &parsed_modules_data);

        // Filter out imports that have alias assignments to avoid redundancy
        let aliased_imports = self.filter_aliased_imports(
            &mut third_party_imports,
            &mut stdlib_imports,
            &ast_rewriter,
        );

        // Add future imports at the very top (after shebang/comments)
        self.add_future_imports_to_bundle(&mut bundle_ast);

        // Add preserved imports at the top
        self.add_preserved_imports_to_bundle(&mut bundle_ast, stdlib_imports, third_party_imports);

        // Add aliased imports separately (not in preserved imports section)
        self.add_aliased_imports_to_bundle(&mut bundle_ast, aliased_imports);

        // Process each module in dependency order using AST transformations
        // First process non-init modules, then init modules to ensure dependencies are available
        let (non_init_modules, init_modules): (Vec<&ModuleNode>, Vec<&ModuleNode>) = modules
            .iter()
            .filter(|m| m.name != entry_module)
            .partition(|m| !self.init_modules.contains(&m.name));

        // Process non-init modules first
        for module in non_init_modules.into_iter().chain(init_modules.into_iter()) {
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
                &import_strategies,
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
                        "Parsed data missing for entry module `{}` at path `{}`. \
                    Ensure the module was parsed and included in parsed_modules_data",
                        entry_module,
                        entry_module_node.path.display()
                    )
                })?;

            // Process the entry module and get a transformed AST
            let mut entry_ast = self.process_module_ast_to_ast(
                &entry_module_node.name,
                parsed_data,
                &ImportStrategy::Dependency,
                &mut ast_rewriter,
                &import_strategies,
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
        Ok(crate::util::normalize_line_endings(bundled_code))
    }

    /// Determine if this is a true module import vs a variable import from a module
    ///
    /// This distinguishes between:
    /// - `from package import module` (where module.py exists) -> true module import
    /// - `from package.module import variable` -> variable import, not a module import
    fn is_true_module_import(
        &mut self,
        import_alias: &crate::ast_rewriter::ImportAlias,
        full_module_name: &str,
    ) -> bool {
        // Check if the full module name resolves to a module path
        let resolves_to_module = self
            .resolver
            .resolve_module_path(full_module_name)
            .unwrap_or(None)
            .is_some();

        log::debug!(
            "is_true_module_import: full_module_name='{}', resolves_to_module={}",
            full_module_name,
            resolves_to_module
        );

        if !resolves_to_module {
            return false;
        }

        // Check if the base module (without the imported name) resolves to a path
        let base_module_path = self
            .resolver
            .resolve_module_path(&import_alias.module_name)
            .unwrap_or(None);

        log::debug!(
            "is_true_module_import: import_alias.module_name='{}', base_module_path={:?}",
            import_alias.module_name,
            base_module_path
        );

        let result = if let Some(base_path) = base_module_path {
            // Check if the base module is a package (__init__.py) vs a specific module (.py)
            let is_package = base_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name == "__init__.py")
                .unwrap_or(false);

            log::debug!(
                "is_true_module_import: base_path={:?}, is_package={}",
                base_path,
                is_package
            );

            if is_package {
                // Base module is a package (__init__.py), importing from a package
                // We need to check if the imported name is defined in the __init__.py file
                // If it is, then it's a variable import; if not, it's a module import

                // Check if the imported name is defined in the __init__.py file
                let is_variable_import =
                    self.is_name_defined_in_package(&base_path, &import_alias.original_name);

                // If defined in __init__.py, it's a variable import; otherwise, it's a module import
                !is_variable_import
            } else {
                // Base module is a specific module file (.py), so we're importing from that module
                // The imported name is likely a variable/function/class from that module
                false
            }
        } else {
            // Base module doesn't resolve to any path, but the full module name does
            // This means we're importing a module from a directory that's not a package
            // For example: "from greetings import greeting" where greetings/ has no __init__.py
            // In this case, it's still a module import
            true
        };

        log::debug!(
            "is_true_module_import: result={} for '{}.{}'",
            result,
            import_alias.module_name,
            import_alias.original_name
        );

        result
    }

    /// Check if a name is defined in a package's __init__.py file
    ///
    /// This is a wrapper around `check_if_name_defined_in_init_py` that handles
    /// errors gracefully and provides logging.
    fn is_name_defined_in_package(&self, init_py_path: &std::path::Path, name: &str) -> bool {
        match self.check_if_name_defined_in_init_py(init_py_path, name) {
            Ok(is_defined) => {
                log::debug!(
                    "is_name_defined_in_package: name='{}', path={:?}, is_defined={}",
                    name,
                    init_py_path,
                    is_defined
                );
                is_defined
            }
            Err(err) => {
                log::warn!(
                    "Failed to check if name '{}' is defined in {:?}: {}",
                    name,
                    init_py_path,
                    err
                );
                // On error, assume it's not defined (conservative approach)
                false
            }
        }
    }

    /// Check if a name is defined in an __init__.py file
    ///
    /// This parses the __init__.py file and checks if the given name is defined as:
    /// - A variable assignment (e.g., `config = ...`)
    /// - A function definition (e.g., `def config():`)
    /// - A class definition (e.g., `class config:`)
    /// - An import that creates the name (e.g., `from .module import config`)
    fn check_if_name_defined_in_init_py(
        &self,
        init_py_path: &std::path::Path,
        name: &str,
    ) -> Result<bool> {
        use std::fs;

        // Read the __init__.py file
        let source = fs::read_to_string(init_py_path)
            .with_context(|| format!("Failed to read __init__.py file: {:?}", init_py_path))?;

        // Normalize line endings
        let source = crate::util::normalize_line_endings(source);

        // Parse into AST
        let parsed = ruff_python_parser::parse_module(&source)
            .with_context(|| format!("Failed to parse __init__.py file: {:?}", init_py_path))?;

        // Check each statement to see if it defines the name
        for stmt in &parsed.syntax().body {
            if self.statement_defines_name(stmt, name) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Check if a statement defines the given name
    fn statement_defines_name(&self, stmt: &Stmt, name: &str) -> bool {
        match stmt {
            // Variable assignments: config = ...
            Stmt::Assign(assign_stmt) => assign_stmt
                .targets
                .iter()
                .any(|target| Self::expression_defines_name(target, name)),

            // Function definitions: def config():
            Stmt::FunctionDef(func_stmt) => func_stmt.name.as_str() == name,

            // Class definitions: class config:
            Stmt::ClassDef(class_stmt) => class_stmt.name.as_str() == name,

            // Import statements: import config
            Stmt::Import(import_stmt) => {
                import_stmt.names.iter().any(|alias| {
                    // Check if this import creates the name (either directly or via alias)
                    let imported_name = alias.name.as_str();
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map(|n| n.as_str())
                        .unwrap_or(imported_name);
                    local_name == name
                })
            }

            // From imports: from .module import config, from .config import config
            Stmt::ImportFrom(import_from_stmt) => import_from_stmt.names.iter().any(|alias| {
                let imported_name = alias.name.as_str();
                let local_name = alias
                    .asname
                    .as_ref()
                    .map(|n| n.as_str())
                    .unwrap_or(imported_name);
                local_name == name
            }),

            _ => false,
        }
    }

    /// Check if an expression defines the given name (for assignment targets)
    fn expression_defines_name(expr: &ruff_python_ast::Expr, name: &str) -> bool {
        use ruff_python_ast::{Expr, ExprList, ExprName, ExprTuple};

        match expr {
            // Simple name: config = ...
            Expr::Name(ExprName { id, .. }) => id.as_str() == name,

            // Tuple assignment: (config, other) = ...
            Expr::Tuple(ExprTuple { elts, .. }) => elts
                .iter()
                .any(|elt| Self::expression_defines_name(elt, name)),

            // List assignment: [config, other] = ...
            Expr::List(ExprList { elts, .. }) => elts
                .iter()
                .any(|elt| Self::expression_defines_name(elt, name)),

            _ => false,
        }
    }

    /// Process a single module's AST to produce a transformed AST for bundling
    #[allow(clippy::too_many_arguments)]
    fn process_module_ast_to_ast(
        &mut self,
        module_name: &str,
        parsed_data: &ParsedModuleData,
        import_strategy: &ImportStrategy,
        ast_rewriter: &mut AstRewriter,
        import_strategies: &IndexMap<String, ImportStrategy>,
    ) -> Result<ModModule> {
        log::info!("Processing module AST '{}'", module_name);

        // Create a transformed AST by cloning the original
        let mut transformed_ast = parsed_data.ast.clone();

        // Apply AST rewriting for name conflict resolution FIRST
        ast_rewriter.rewrite_module_ast(module_name, &mut transformed_ast)?;

        // Note: Bundled variables are now collected upfront in emit_bundle, not here

        // Apply import alias transformations BEFORE removing imports
        // This ensures that alias information is available when transforming expressions
        ast_rewriter.transform_module_ast(&mut transformed_ast)?;

        // Transform relative imports in modules BEFORE removing imports
        // This ensures relative imports are resolved to bundled variable references
        let bundled_modules = self.create_bundled_modules_mapping();
        log::debug!(
            "Bundled modules mapping for {}: {:?}",
            module_name,
            bundled_modules
        );
        ast_rewriter.transform_init_py_relative_imports(
            module_name,
            &mut transformed_ast,
            &bundled_modules,
        )?;

        // For __init__.py files, also transform absolute first-party imports to use bundled variables
        if self.is_init_py_module(module_name) {
            self.transform_absolute_first_party_imports_in_init_py(TransformImportsParams {
                module_name,
                module_ast: &mut transformed_ast,
                import_strategies,
            })?;
        }

        // Note: Global declarations for imports will be handled in create_module_exec_statement
        // when the code is converted to string for exec

        // Remove first-party imports and unused imports AFTER transformations
        // For packages (__init__.py files) using ModuleImport strategy, we need to be more careful
        // about which imports to remove, as some may be essential for the package to function
        if matches!(import_strategy, ImportStrategy::ModuleImport)
            && self.is_init_py_module(module_name)
        {
            // For __init__.py files with ModuleImport strategy, only remove unused imports
            // Don't remove first-party imports as they may be essential for package functionality
            self.remove_unused_imports(&mut transformed_ast, &parsed_data.unused_imports)?;
        } else {
            // For regular modules, remove both first-party and unused imports as before
            self.remove_first_party_imports(
                &mut transformed_ast,
                &parsed_data.first_party_imports,
            )?;
            self.remove_unused_imports(&mut transformed_ast, &parsed_data.unused_imports)?;
        }

        // Apply bundling strategy based on how this module is imported
        match import_strategy {
            ImportStrategy::ModuleImport => {
                // For modules imported as "import module", create a module namespace using AST nodes
                let module_ast = ModModule {
                    body: self.create_module_namespace_ast(ModuleNamespaceParams {
                        module_name,
                        module_ast: &transformed_ast,
                        ast_rewriter,
                        import_strategy,
                    })?,
                    range: Default::default(),
                };
                Ok(module_ast)
            }
            ImportStrategy::FromImport | ImportStrategy::Dependency => {
                // For modules imported as "from module import" or dependency modules,
                // add variable exposure statements and return the transformed AST
                let exposure_statements =
                    self.create_variable_exposure_statements(module_name, ast_rewriter)?;
                let exposure_count = exposure_statements.len();
                if !exposure_statements.is_empty() {
                    transformed_ast.body.extend(exposure_statements);
                    log::debug!(
                        "Added {} variable exposure statements to module '{}'",
                        exposure_count,
                        module_name
                    );
                }
                Ok(transformed_ast)
            }
        }
    }

    /// Create a module namespace using AST operations
    fn create_module_namespace_ast(&mut self, params: ModuleNamespaceParams) -> Result<Vec<Stmt>> {
        // Start with the types import and namespace creation
        let mut namespace_stmts = self.create_module_namespace_structure(params.module_name)?;

        // Convert the module AST to a string for the exec call
        // This is necessary because Python AST doesn't allow directly representing a module as an expression
        let module_code = {
            let empty_parsed = ruff_python_parser::parse_module("")?;
            let stylist = Stylist::from_tokens(empty_parsed.tokens(), "");

            // Generate code for each statement and combine them
            let mut code_parts = Vec::new();
            for stmt in &params.module_ast.body {
                let generator = Generator::from(&stylist);
                let stmt_code = generator.stmt(stmt);
                code_parts.push(stmt_code);
            }

            crate::util::normalize_line_endings(code_parts.join("\n"))
        };

        // Add the exec call that will execute the module code in its namespace
        // For __init__.py files and modules with bundled variable references, include globals to access bundled variables
        let needs_globals = self.is_init_py_module(params.module_name)
            || self.module_has_bundled_variable_references(&module_code);
        let exec_stmt = self.create_module_exec_statement(ModuleExecParams {
            module_name: params.module_name,
            module_code: &module_code,
            include_globals: needs_globals,
            import_strategy: params.import_strategy,
        })?;
        namespace_stmts.push(exec_stmt);

        // Add exposure statements for conflict-renamed variables that need to be accessible on the module object
        let module_exposure_stmts =
            self.create_module_exposure_statements(params.module_name, params.ast_rewriter)?;
        namespace_stmts.extend(module_exposure_stmts);

        // For ModuleImport strategy packages with global declarations, we need to expose
        // the globally declared variables as module attributes
        // This is handled inside the exec call by modifying the module code itself

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
                    range: TextRange::default(),
                }],
                range: TextRange::default(),
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
            Self::build_dotted_name_expr(&parts)
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

    /// Check if a module name represents an __init__.py file
    fn is_init_py_module(&self, module_name: &str) -> bool {
        // Check if this module was identified as coming from an __init__.py file
        // during the parsing phase
        self.init_modules.contains(module_name)
    }

    /// Check if module code contains references to bundled variables from other modules
    fn module_has_bundled_variable_references(&self, module_code: &str) -> bool {
        // Look for bundled variable patterns like "__module_name_variable"
        // These indicate that the module references variables from other modules
        BUNDLED_VAR_PATTERN.is_match(module_code)
    }

    /// Check if a variable name follows the bundled variable pattern
    fn is_bundled_variable_name(&self, name: &str) -> bool {
        // Bundled variables follow the pattern: __module_name_variable
        // This matches the regex used in BUNDLED_VAR_PATTERN
        BUNDLED_VAR_PATTERN.is_match(name)
    }

    /// Process a single rename entry and create exposure statement if appropriate
    fn process_rename_entry(
        &self,
        rename_entry: (&str, &str), // (original_name, renamed_name)
        module_name: &str,
        ast_rewriter: &crate::ast_rewriter::AstRewriter,
    ) -> Option<Stmt> {
        let (original_name, renamed_name) = rename_entry;

        log::debug!(
            "process_rename_entry: module='{}', original='{}', renamed='{}', is_conflict={}",
            module_name,
            original_name,
            renamed_name,
            ast_rewriter.is_conflict_based_rename(original_name, module_name)
        );

        // If the original name and renamed name are the same, no exposure needed
        if original_name == renamed_name {
            log::debug!(
                "Skipping exposure for non-renamed variable: {} in {}",
                original_name,
                module_name
            );
            return None;
        }

        // For conflict-based renames, skip exposure statements to avoid invalid references
        // When variables are renamed due to conflicts, the original names no longer exist
        // in the module's namespace, so we can't create backward-compatible aliases
        if ast_rewriter.is_conflict_based_rename(original_name, module_name) {
            log::debug!(
                "Skipping exposure for conflict-based rename: {} -> {} in {}",
                original_name,
                renamed_name,
                module_name
            );
            return None;
        }

        // For non-conflict renames, determine the correct direction
        // For bundled variables (with module prefixes), use: renamed_name = original_name
        // For special variables like __all__, use: original_name = renamed_name
        let (target, source) = if self.is_bundled_variable_name(renamed_name) {
            // Bundled variable: __module_name_var = var
            (renamed_name, original_name)
        } else {
            // Special variable or simple rename: var = __module_name_var
            (original_name, renamed_name)
        };

        let assignment = Stmt::Assign(StmtAssign {
            targets: vec![Expr::Name(ExprName {
                id: target.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                id: source.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        });

        log::debug!(
            "Creating exposure statement: {} = {} (in module '{}')",
            target,
            source,
            module_name
        );

        Some(assignment)
    }

    /// Process all renames for a module and collect statements
    fn process_module_renames(
        &self,
        renames: &IndexMap<String, String>,
        module_name: &str,
        ast_rewriter: &crate::ast_rewriter::AstRewriter,
    ) -> Vec<Stmt> {
        let mut statements = Vec::new();
        for (original_name, renamed_name) in renames {
            if let Some(statement) =
                self.process_rename_entry((original_name, renamed_name), module_name, ast_rewriter)
            {
                statements.push(statement);
            }
        }
        statements
    }

    /// Create exposure statements for renamed variables in a module
    fn create_variable_exposure_statements(
        &self,
        module_name: &str,
        ast_rewriter: &crate::ast_rewriter::AstRewriter,
    ) -> Result<Vec<Stmt>> {
        let mut statements = Vec::new();

        // Get renamed variables for this module from bundled_variables
        if let Some(renames) = self.bundled_variables.get(module_name) {
            log::debug!(
                "Creating exposure statements for module '{}' with {} renames",
                module_name,
                renames.len()
            );

            statements.extend(self.process_module_renames(renames, module_name, ast_rewriter));
        } else {
            log::debug!("No renamed variables found for module '{}'", module_name);
        }

        Ok(statements)
    }

    /// Create exposure statements for conflict-renamed variables that need to be accessible on module objects
    /// This handles cases where variables with conflict-based renames inside exec namespaces need to be exposed
    /// as attributes on the module object (e.g., greetings.greeting.message = __greetings_greeting_message)
    fn create_module_exposure_statements(
        &self,
        module_name: &str,
        ast_rewriter: &AstRewriter,
    ) -> Result<Vec<Stmt>> {
        let mut statements = Vec::new();

        // Get renamed variables for this module
        if let Some(renames) = self.bundled_variables.get(module_name) {
            log::debug!(
                "Creating module exposure statements for '{}' with {} renames",
                module_name,
                renames.len()
            );

            statements.extend(self.create_conflict_exposure_statements(
                module_name,
                renames,
                ast_rewriter,
            )?);
        }

        Ok(statements)
    }

    /// Create exposure statements for conflict-based renames in a module
    fn create_conflict_exposure_statements(
        &self,
        module_name: &str,
        renames: &IndexMap<String, String>,
        ast_rewriter: &AstRewriter,
    ) -> Result<Vec<Stmt>> {
        let mut statements = Vec::new();

        for (original_name, renamed_name) in renames {
            // Only create exposure statements for conflict-based renames
            if ast_rewriter.is_conflict_based_rename(original_name, module_name) {
                let exposure_stmt = self.create_module_attribute_assignment(
                    module_name,
                    original_name,
                    renamed_name,
                )?;

                log::debug!(
                    "Created module exposure: {}.{} = {}",
                    module_name,
                    original_name,
                    renamed_name
                );

                statements.push(exposure_stmt);
            }
        }

        Ok(statements)
    }

    /// Create a module attribute assignment statement (e.g., greetings.greeting.message = __greetings_greeting_message)
    fn create_module_attribute_assignment(
        &self,
        module_name: &str,
        original_name: &str,
        renamed_name: &str,
    ) -> Result<Stmt> {
        // Create the target expression: module_name.original_name
        let target_expr = if module_name.contains('.') {
            let parts: Vec<&str> = module_name.split('.').collect();
            let mut module_expr = Self::create_dotted_module_expr(&parts, ExprContext::Load);
            module_expr =
                Self::create_attribute_expr(module_expr, original_name, ExprContext::Store);
            module_expr
        } else {
            Self::create_attribute_expr(
                Self::create_name_expr(module_name, ExprContext::Load),
                original_name,
                ExprContext::Store,
            )
        };

        // Create the source expression: getattr(module_name, renamed_name)
        let source_expr = if module_name.contains('.') {
            let parts: Vec<&str> = module_name.split('.').collect();
            let module_expr = Self::create_dotted_module_expr(&parts, ExprContext::Load);
            Self::create_getattr_call(module_expr, renamed_name)
        } else {
            let module_expr = Self::create_name_expr(module_name, ExprContext::Load);
            Self::create_getattr_call(module_expr, renamed_name)
        };

        // Create the assignment statement
        Ok(Stmt::Assign(StmtAssign {
            targets: vec![target_expr],
            value: Box::new(source_expr),
            range: TextRange::default(),
        }))
    }

    /// Create an AST for the exec statement that executes module code in its namespace
    fn create_module_exec_statement(&self, params: ModuleExecParams) -> Result<Stmt> {
        // Apply global declarations for packages using ModuleImport strategy
        let final_code = if matches!(params.import_strategy, ImportStrategy::ModuleImport)
            && self.is_init_py_module(params.module_name)
        {
            self.add_global_declarations_to_code(params.module_name, params.module_code)
        } else {
            params.module_code.to_string()
        };

        // Create the module code string literal
        let code_literal = Expr::StringLiteral(ruff_python_ast::ExprStringLiteral {
            value: ruff_python_ast::StringLiteralValue::single(ruff_python_ast::StringLiteral {
                range: TextRange::default(),
                value: final_code.into_boxed_str(),
                flags: ruff_python_ast::StringLiteralFlags::empty(),
            }),
            range: TextRange::default(),
        });

        // Create the module __dict__ expression
        let module_dict = Expr::Attribute(ExprAttribute {
            value: Box::new(if params.module_name.contains('.') {
                // Handle dotted module names
                let parts: Vec<&str> = params.module_name.split('.').collect();
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
                    id: Identifier::new(params.module_name, TextRange::default()).into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })
            }),
            attr: Identifier::new("__dict__", TextRange::default()),
            ctx: ExprContext::Load,
            range: TextRange::default(),
        });

        // Create the arguments based on whether globals are needed
        let args = if params.include_globals {
            // For package __init__.py files that reference their own submodules,
            // we need to make the package module available in the exec globals
            if self.is_init_py_module(params.module_name)
                && params.module_code.contains(params.module_name)
            {
                // Create a dict merge: {**globals(), module_name: module}
                // This is done by creating dict(globals(), **{module_name: module})
                let module_ref = if params.module_name.contains('.') {
                    // For nested modules, use just the top-level package name
                    let parts: Vec<&str> = params.module_name.split('.').collect();
                    Expr::Name(ExprName {
                        id: parts[0].into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })
                } else {
                    Expr::Name(ExprName {
                        id: params.module_name.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })
                };

                // Create dict merge compatible with Python 3.5+
                // Use {**globals(), 'package_name': package} pattern
                let package_name = if params.module_name.contains('.') {
                    params
                        .module_name
                        .split('.')
                        .next()
                        .expect("module name should have at least one part before dot")
                        .to_string()
                } else {
                    params.module_name.to_string()
                };

                let dict_merge = Expr::Dict(ruff_python_ast::ExprDict {
                    items: vec![
                        ruff_python_ast::DictItem {
                            key: None, // **kwargs unpacking
                            value: Expr::Call(ExprCall {
                                func: Box::new(Expr::Name(ExprName {
                                    id: "globals".into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                arguments: ruff_python_ast::Arguments {
                                    args: vec![].into(),
                                    keywords: vec![].into(),
                                    range: TextRange::default(),
                                },
                                range: TextRange::default(),
                            }),
                        },
                        ruff_python_ast::DictItem {
                            key: Some(Expr::StringLiteral(ruff_python_ast::ExprStringLiteral {
                                value: ruff_python_ast::StringLiteralValue::single(
                                    ruff_python_ast::StringLiteral {
                                        range: TextRange::default(),
                                        value: package_name.into_boxed_str(),
                                        flags: ruff_python_ast::StringLiteralFlags::empty(),
                                    },
                                ),
                                range: TextRange::default(),
                            })),
                            value: module_ref,
                        },
                    ],
                    range: TextRange::default(),
                });

                vec![code_literal, dict_merge, module_dict]
            } else {
                // exec(code, globals(), module.__dict__)
                vec![
                    code_literal,
                    // globals() call
                    Expr::Call(ExprCall {
                        func: Box::new(Expr::Name(ExprName {
                            id: Identifier::new("globals", TextRange::default()).into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        arguments: Arguments {
                            args: vec![].into(),
                            keywords: vec![].into(),
                            range: TextRange::default(),
                        },
                        range: TextRange::default(),
                    }),
                    module_dict,
                ]
            }
        } else {
            // exec(code, module.__dict__)
            vec![code_literal, module_dict]
        };

        // Create the exec call
        let exec_call = Expr::Call(ExprCall {
            func: Box::new(Expr::Name(ExprName {
                id: Identifier::new("exec", TextRange::default()).into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: Arguments {
                args: args.into(),
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
        parsed_modules_data: &IndexMap<std::path::PathBuf, ParsedModuleData>,
    ) -> Result<IndexMap<String, ImportStrategy>> {
        let mut strategies = IndexMap::new();

        // Find the entry module data
        let entry_module_node =
            modules
                .iter()
                .find(|m| m.name == entry_module)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Entry module `{}` not found. Available modules: {:?}",
                        entry_module,
                        modules.iter().map(|m| &m.name).collect::<Vec<_>>()
                    )
                })?;

        let entry_parsed_data = parsed_modules_data
            .get(&entry_module_node.path)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Parsed data missing for entry module `{}` at path `{}`. \
                Ensure the module was parsed and included in parsed_modules_data",
                    entry_module,
                    entry_module_node.path.display()
                )
            })?;

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

        // Also analyze imports in __init__.py files to determine if they need module namespaces
        // This is important for relative imports like "from . import greeting" in __init__.py
        for module in modules {
            if self.init_modules.contains(&module.name) {
                self.process_init_module_strategies(module, parsed_modules_data, &mut strategies);
            }
        }

        // Ensure that packages (modules with __init__.py) always use ModuleImport strategy
        // This is necessary because packages need to execute their __init__.py code in a namespace
        for module in modules {
            if self.init_modules.contains(&module.name) && strategies.contains_key(&module.name) {
                strategies.insert(module.name.clone(), ImportStrategy::ModuleImport);
            }
        }

        Ok(strategies)
    }

    /// Process import strategies for a single __init__.py module
    fn process_init_module_strategies(
        &self,
        module: &crate::dependency_graph::ModuleNode,
        parsed_modules_data: &IndexMap<std::path::PathBuf, ParsedModuleData>,
        strategies: &mut IndexMap<String, ImportStrategy>,
    ) {
        if let Some(init_parsed_data) = parsed_modules_data.get(&module.path) {
            log::debug!("Analyzing imports in __init__.py module: {}", module.name);
            self.process_init_module_statements(
                &init_parsed_data.ast.body,
                &module.name,
                strategies,
            );
        }
    }

    /// Process import statements within an __init__.py module
    fn process_init_module_statements(
        &self,
        statements: &[Stmt],
        module_name: &str,
        strategies: &mut IndexMap<String, ImportStrategy>,
    ) {
        for stmt in statements {
            match stmt {
                Stmt::Import(import_stmt) => {
                    self.process_import_strategies(import_stmt, strategies);
                }
                Stmt::ImportFrom(import_from_stmt) => {
                    self.process_init_py_import_from_strategies(
                        import_from_stmt,
                        module_name,
                        strategies,
                    );
                }
                _ => {}
            }
        }
    }

    /// Process import statements for strategy analysis
    fn process_import_strategies(
        &self,
        import_stmt: &StmtImport,
        strategies: &mut IndexMap<String, ImportStrategy>,
    ) {
        // `import module` - needs namespace
        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();
            if self.is_first_party_module(module_name) {
                strategies.insert(module_name.to_string(), ImportStrategy::ModuleImport);

                // Also set parent packages as ModuleImport for submodule imports
                // For example, if importing "greetings.irrelevant", also set "greetings" as ModuleImport
                self.set_parent_packages_as_module_import(module_name, strategies);
            }
        }
    }

    /// Set parent packages as ModuleImport strategy for submodule imports
    /// This ensures that __init__.py files are properly executed in their namespace
    fn set_parent_packages_as_module_import(
        &self,
        module_name: &str,
        strategies: &mut IndexMap<String, ImportStrategy>,
    ) {
        let parts: Vec<&str> = module_name.split('.').collect();

        // For each parent package level, set it as ModuleImport if it's a first-party module
        for i in 1..parts.len() {
            let parent_module = parts[..i].join(".");

            if self.is_first_party_module(&parent_module)
                && !strategies.contains_key(&parent_module)
            {
                strategies.insert(parent_module.clone(), ImportStrategy::ModuleImport);
            }
        }
    }

    /// Process import-from statements for strategy analysis
    fn process_import_from_strategies(
        &self,
        import_from_stmt: &StmtImportFrom,
        strategies: &mut IndexMap<String, ImportStrategy>,
    ) {
        if let Some(module) = &import_from_stmt.module {
            let module_name = module.as_str();
            if !self.is_first_party_module(module_name) {
                return;
            }
            strategies.insert(module_name.to_string(), ImportStrategy::FromImport);

            for alias in &import_from_stmt.names {
                let imported_name = alias.name.as_str();
                let full_module_name = format!("{}.{}", module_name, imported_name);
                self.insert_module_import_strategy(&full_module_name, strategies);
            }
        }
    }

    /// Process import-from statements in __init__.py files for strategy analysis
    /// This handles relative imports like "from . import greeting" in __init__.py
    fn process_init_py_import_from_strategies(
        &self,
        import_from_stmt: &StmtImportFrom,
        init_module_name: &str,
        strategies: &mut IndexMap<String, ImportStrategy>,
    ) {
        // Handle relative imports in __init__.py files
        if import_from_stmt.level > 0 {
            // This is a relative import like "from . import greeting"
            for alias in &import_from_stmt.names {
                let imported_name = alias.name.as_str();

                // Determine the target module path and set import strategy
                let target_module = self.resolve_relative_import_target_module(
                    import_from_stmt,
                    init_module_name,
                    imported_name,
                );

                self.set_target_module_strategy(&target_module, init_module_name, strategies);
            }
        } else {
            // Non-relative imports in __init__.py - process normally
            self.process_import_from_strategies(import_from_stmt, strategies);
        }
    }

    /// Resolve the target module path for a relative import
    fn resolve_relative_import_target_module(
        &self,
        import_from_stmt: &StmtImportFrom,
        init_module_name: &str,
        imported_name: &str,
    ) -> String {
        // For "from . import greeting" in greetings/__init__.py,
        // the target module is "greetings.greeting"
        // For "from .greeting import message" in greetings/__init__.py,
        // the target module is also "greetings.greeting"
        if let Some(module_ref) = &import_from_stmt.module {
            // "from .subpkg import module" case - target is the subpackage
            let module_name = module_ref.as_str();
            format!("{}.{}", init_module_name, module_name)
        } else {
            // "from . import module" case - target is the sibling module
            format!("{}.{}", init_module_name, imported_name)
        }
    }

    /// Set the import strategy for a target module if it's first-party
    fn set_target_module_strategy(
        &self,
        target_module: &str,
        init_module_name: &str,
        strategies: &mut IndexMap<String, ImportStrategy>,
    ) {
        if self.is_first_party_module(target_module) {
            // Set the imported module as ModuleImport so it gets its own namespace
            // This is necessary because the __init__.py will reference it as a module object
            log::debug!(
                "Setting {} as ModuleImport due to relative import in __init__.py {}",
                target_module,
                init_module_name
            );
            strategies.insert(target_module.to_string(), ImportStrategy::ModuleImport);
        }
    }

    fn insert_module_import_strategy(
        &self,
        full_module_name: &str,
        strategies: &mut IndexMap<String, ImportStrategy>,
    ) {
        if self.is_first_party_module(full_module_name) {
            strategies.insert(full_module_name.to_string(), ImportStrategy::ModuleImport);
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
        first_party_imports: &IndexSet<String>,
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
        unused_imports: &IndexSet<String>,
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
            let module_name = module.as_str();
            // Always remove __future__ imports as they are hoisted to the top
            if module_name == "__future__" {
                return false;
            }
            keep_predicate(module_name)
        } else {
            // Relative import - keep for now (could be refined later)
            true
        }
    }

    /// Collect first-party imports from AST instead of re-parsing source
    fn collect_first_party_imports_from_ast(&self, module: &ModModule) -> Result<IndexSet<String>> {
        let mut first_party_imports = IndexSet::new();

        for stmt in &module.body {
            self.collect_first_party_from_statement(stmt, &mut first_party_imports);
        }

        log::debug!("Collected first-party imports: {:?}", first_party_imports);
        Ok(first_party_imports)
    }

    /// Extract first-party imports from an AST statement
    fn collect_first_party_from_statement(&self, stmt: &Stmt, imports: &mut IndexSet<String>) {
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
        imports: &mut IndexSet<String>,
    ) {
        // Handle relative imports (e.g., `from . import x`) as first-party
        if import_from_stmt.module.is_none() {
            if import_from_stmt.level > 0 {
                // Insert empty string marker for relative import
                imports.insert(String::new());
            }
            return;
        }
        let module = import_from_stmt
            .module
            .as_ref()
            .expect("module should be present for non-relative imports");

        let module_name = module.as_str();
        if matches!(
            self.resolver.classify_import(module_name),
            ImportType::FirstParty
        ) {
            imports.insert(module_name.to_string());
        }
    }

    /// Collect future imports from AST
    fn collect_future_imports_from_ast(&mut self, module: &ModModule) {
        for stmt in &module.body {
            let Stmt::ImportFrom(import_from_stmt) = stmt else {
                continue;
            };

            let Some(module) = &import_from_stmt.module else {
                continue;
            };

            if module.as_str() != "__future__" {
                continue;
            }

            // Collect all features imported from __future__
            for alias in &import_from_stmt.names {
                let feature_name = alias.name.as_str();
                self.future_imports.insert(feature_name.to_string());
            }
        }
    }

    /// Helper to collect first-party imports from regular import statements
    fn collect_first_party_from_import(
        &self,
        import_stmt: &StmtImport,
        imports: &mut IndexSet<String>,
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
        let mut third_party_imports = IndexSet::new();

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
        third_party_imports: &mut IndexSet<String>,
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
        let comment_content = format!("__CRIBO_COMMENT_MARKER__{}", comment);

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
        if trimmed.starts_with("'__CRIBO_COMMENT_MARKER__") && trimmed.ends_with("'") {
            // Extract the comment content
            let start_idx = "'__CRIBO_COMMENT_MARKER__".len();
            let content = &trimmed[start_idx..trimmed.len() - 1]; // Skip marker and quotes
            content.to_string()
        } else if trimmed.starts_with("\"__CRIBO_COMMENT_MARKER__") && trimmed.ends_with("\"") {
            // Extract the comment content (double quotes)
            let start_idx = "\"__CRIBO_COMMENT_MARKER__".len();
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
        stdlib_imports: IndexSet<String>,
        third_party_imports: IndexSet<String>,
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

    /// Create a mapping of module names to their bundled variable names
    /// This is used for transforming relative imports in __init__.py files
    fn create_bundled_modules_mapping(&self) -> IndexMap<String, String> {
        let mut mapping = IndexMap::new();

        // Sort module names for deterministic processing order
        let mut sorted_modules: Vec<_> = self.bundled_variables.iter().collect();
        sorted_modules.sort_by_key(|(module_name, _)| *module_name);

        for (module_name, variables) in sorted_modules {
            // Sort variable names within each module for deterministic order
            let mut sorted_variables: Vec<_> = variables.iter().collect();
            sorted_variables.sort_by_key(|(original_name, _)| *original_name);

            for (original_name, bundled_name) in sorted_variables {
                // Map "module.original_name" -> "bundled_name"
                let key = format!("{}.{}", module_name, original_name);
                mapping.insert(key, bundled_name.clone());
            }
        }
        mapping
    }

    /// Track a bundled variable for a module
    fn track_bundled_variable(
        &mut self,
        module_name: &str,
        original_name: &str,
        bundled_name: &str,
    ) {
        self.bundled_variables
            .entry(module_name.to_string())
            .or_default()
            .insert(original_name.to_string(), bundled_name.to_string());
    }

    /// Collect bundled variables from the AST rewriter
    fn collect_bundled_variables_from_rewriter(
        &mut self,
        module_name: &str,
        ast_rewriter: &AstRewriter,
    ) {
        // Get all module-level symbols (both renamed and original)
        let module_symbols = ast_rewriter.get_module_symbols(module_name);

        log::debug!(
            "Found {} symbols for module {}: {:?}",
            module_symbols.len(),
            module_name,
            module_symbols
        );

        for (original_name, final_name) in module_symbols {
            self.track_bundled_variable(module_name, &original_name, &final_name);
        }
    }

    /// Helper to build a nested dotted name expression for assignment target
    fn build_dotted_name_expr(parts: &[&str]) -> Expr {
        let mut current_expr = Expr::Name(ExprName {
            id: Identifier::new(parts[0], TextRange::default()).into(),
            ctx: ExprContext::Load,
            range: TextRange::default(),
        });
        let len = parts.len();
        for (idx, &part) in parts[1..].iter().enumerate() {
            let ctx = if idx + 2 == len {
                ExprContext::Store
            } else {
                ExprContext::Load
            };
            current_expr = Expr::Attribute(ExprAttribute {
                value: Box::new(current_expr),
                attr: Identifier::new(part, TextRange::default()),
                ctx,
                range: TextRange::default(),
            });
        }
        current_expr
    }

    /// Filter out redundant general imports based on specific imports found in module data
    /// This version analyzes the original parsed modules before bundling
    fn filter_redundant_imports_from_modules(
        &self,
        imports: IndexSet<String>,
        parsed_modules_data: &IndexMap<std::path::PathBuf, ParsedModuleData>,
    ) -> IndexSet<String> {
        let mut filtered_imports = IndexSet::new();

        // Collect all specific imports from all module ASTs
        let mut specific_imports = IndexSet::new();
        for parsed_data in parsed_modules_data.values() {
            for stmt in &parsed_data.ast.body {
                self.collect_specific_imports_from_statement(stmt, &mut specific_imports);
            }
        }

        log::debug!("Found specific imports: {:?}", specific_imports);

        // Filter out general imports that have specific imports
        for import in imports {
            let has_specific_imports = specific_imports.iter().any(|specific_import| {
                // Check if this general import has corresponding specific imports
                // Use exact module name matching to avoid substring false positives
                specific_import.split('.').next() == Some(&import)
            });

            if !has_specific_imports {
                filtered_imports.insert(import);
            } else {
                log::debug!(
                    "Filtering out redundant general import '{}' due to specific imports",
                    import
                );
            }
        }

        filtered_imports
    }

    /// Collect specific imports from a statement (e.g., "from typing import Dict, Any")
    fn collect_specific_imports_from_statement(
        &self,
        stmt: &Stmt,
        specific_imports: &mut IndexSet<String>,
    ) {
        let Stmt::ImportFrom(import_from_stmt) = stmt else {
            return;
        };
        let Some(module) = &import_from_stmt.module else {
            return;
        };
        let module_name = module.as_str();
        for alias in &import_from_stmt.names {
            let imported_name = alias.name.as_str();
            let full_name = format!("{}.{}", module_name, imported_name);
            specific_imports.insert(full_name);
        }
    }

    /// Add global declarations for imported names in packages using ModuleImport strategy
    /// This ensures that functions defined in the exec can access the imported names
    /// Also adds exposure statements to make the variables accessible as module attributes
    fn add_global_declarations_to_code(&self, _module_name: &str, code: &str) -> String {
        log::debug!("add_global_declarations_to_code called");

        // Parse the Python code to find assignment statements that import from modules
        let imported_names = self.find_imported_names_from_ast(code);

        if imported_names.is_empty() {
            // Fallback to string-based approach if AST parsing fails
            log::debug!(
                "AST parsing failed or found no assignments, falling back to string parsing"
            );
            let imported_names = self.find_imported_names_from_string(code);
            return self.build_code_with_globals(code, imported_names);
        }

        self.build_code_with_globals(code, imported_names)
    }

    /// Find imported variable names using AST parsing for robustness
    fn find_imported_names_from_ast(&self, code: &str) -> Vec<String> {
        let mut imported_names = Vec::new();

        match ruff_python_parser::parse_module(code) {
            Ok(parsed) => {
                for stmt in &parsed.syntax().body {
                    if let Stmt::Assign(assign) = stmt {
                        self.extract_attribute_assignments(assign, &mut imported_names);
                    }
                }
                log::debug!(
                    "AST parsing found {} imported names: {:?}",
                    imported_names.len(),
                    imported_names
                );
            }
            Err(err) => {
                log::debug!(
                    "AST parsing failed: {}, falling back to string approach",
                    err
                );
            }
        }

        imported_names
    }

    /// Extract variable names from assignments that use attribute access
    fn extract_attribute_assignments(&self, assign: &StmtAssign, imported_names: &mut Vec<String>) {
        // Check if RHS is an attribute access (module.attr)
        if let Expr::Attribute(_) = assign.value.as_ref() {
            // Get the target variable name(s)
            for target in &assign.targets {
                if let Expr::Name(name) = target {
                    imported_names.push(name.id.to_string());
                }
            }
        }
    }

    /// Fallback string-based approach for finding imported names
    fn find_imported_names_from_string(&self, code: &str) -> Vec<String> {
        let lines: Vec<&str> = code.lines().collect();
        let mut imported_names = Vec::new();

        for line in &lines {
            let trimmed = line.trim();
            // Look for lines like "sub_function = nested_package.submodule.sub_function"
            if let Some(eq_pos) = trimmed.find(" = ") {
                let var_name = trimmed[..eq_pos].trim();
                let value = trimmed[eq_pos + 3..].trim();

                // Check if it's an attribute access (contains a dot) and valid identifier
                if value.contains('.')
                    && !var_name.is_empty()
                    && var_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    imported_names.push(var_name.to_string());
                }
            }
        }

        imported_names
    }

    /// Build the final code with global declarations and exposure statements
    fn build_code_with_globals(&self, code: &str, imported_names: Vec<String>) -> String {
        if !imported_names.is_empty() {
            log::debug!(
                "Found {} imported names to make global: {:?}",
                imported_names.len(),
                imported_names
            );
            let global_declaration = format!("global {}", imported_names.join(", "));

            // Add the global declaration at the beginning of the code
            let mut result = global_declaration;
            result.push('\n');
            result.push_str(code);

            // Add exposure statements at the end to make variables accessible as module attributes
            for var_name in &imported_names {
                result.push('\n');
                result.push_str(&format!("locals()['{}'] = {}", var_name, var_name));
            }

            result
        } else {
            log::debug!("No imported names found for global declarations");
            code.to_string()
        }
    }

    /// Transform absolute first-party imports in __init__.py files to use bundled variables
    /// This handles cases like "from greetings.english import message" in greetings/__init__.py
    /// where greetings.english has been inlined and "message" is available globally
    fn transform_absolute_first_party_imports_in_init_py(
        &self,
        params: TransformImportsParams,
    ) -> Result<()> {
        log::debug!(
            "Transforming absolute first-party imports in __init__.py for module: {}",
            params.module_name
        );

        let mut statements_to_replace = Vec::new();
        let mut new_statements = Vec::new();

        // Find absolute first-party import statements
        for (i, stmt) in params.module_ast.body.iter().enumerate() {
            if let Stmt::ImportFrom(import_from_stmt) = stmt {
                // Skip relative imports (they're handled elsewhere)
                if import_from_stmt.level > 0 {
                    continue;
                }

                let Some(module) = &import_from_stmt.module else {
                    continue;
                };

                let imported_module = module.as_str();

                // Check if this is a first-party import
                if !self.is_first_party_module(imported_module) {
                    continue;
                }

                log::debug!(
                    "Found absolute first-party import in __init__.py: from {} import [...]",
                    imported_module
                );

                // Transform each imported name to a simple assignment
                for alias in &import_from_stmt.names {
                    let imported_name = alias.name.as_str();
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map(|n| n.as_str())
                        .unwrap_or(imported_name);

                    // Check the import strategy of the source module to determine the correct reference
                    let value_expr = self.create_value_expr_for_strategy(
                        params.import_strategies.get(imported_module),
                        imported_module,
                        imported_name,
                    );

                    let assignment = Stmt::Assign(StmtAssign {
                        targets: vec![Expr::Name(ExprName {
                            id: local_name.into(),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(value_expr),
                        range: TextRange::default(),
                    });

                    new_statements.push(assignment);

                    log::debug!(
                        "Transformed absolute import to assignment: {} = {} (strategy: {:?})",
                        local_name,
                        imported_module,
                        params.import_strategies.get(imported_module)
                    );
                }

                statements_to_replace.push(i);
            }
        }

        // Replace import statements with assignments
        if !statements_to_replace.is_empty() {
            self.replace_import_statements_with_assignments(
                params.module_ast,
                &statements_to_replace,
                new_statements,
            );
            log::debug!(
                "Replaced {} absolute first-party import statements with assignments",
                statements_to_replace.len()
            );
        }

        Ok(())
    }

    /// Create a module attribute expression for ModuleImport strategy
    fn create_module_attribute_expr(&self, imported_module: &str, imported_name: &str) -> Expr {
        if imported_module.contains('.') {
            // Handle dotted module names like "nested_package.utils"
            let parts: Vec<&str> = imported_module.split('.').collect();
            let mut current_expr = Expr::Name(ExprName {
                id: parts[0].into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            });

            // Build the attribute chain for the remaining parts
            for &part in &parts[1..] {
                current_expr = Expr::Attribute(ExprAttribute {
                    value: Box::new(current_expr),
                    attr: Identifier::new(part, TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                });
            }

            // Add the final imported name as an attribute
            Expr::Attribute(ExprAttribute {
                value: Box::new(current_expr),
                attr: Identifier::new(imported_name, TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })
        } else {
            // Simple module name
            Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: imported_module.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(imported_name, TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })
        }
    }

    /// Create a value expression based on import strategy
    fn create_value_expr_for_strategy(
        &self,
        strategy: Option<&ImportStrategy>,
        imported_module: &str,
        imported_name: &str,
    ) -> Expr {
        match strategy {
            Some(ImportStrategy::ModuleImport) => {
                self.create_module_attribute_expr(imported_module, imported_name)
            }
            Some(ImportStrategy::FromImport) | Some(ImportStrategy::Dependency) => {
                // Module was inlined, so imported_name should be available globally
                Expr::Name(ExprName {
                    id: imported_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })
            }
            None => {
                // No strategy info, fallback to simple name
                Expr::Name(ExprName {
                    id: imported_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })
            }
        }
    }

    /// Add replacement statements for an import
    fn add_replacement_statements_for_import(
        &self,
        new_body: &mut Vec<Stmt>,
        import_from_stmt: &ruff_python_ast::StmtImportFrom,
        new_stmt_iter: &mut impl Iterator<Item = Stmt>,
    ) {
        // Add one assignment per imported name
        for _ in &import_from_stmt.names {
            if let Some(assignment) = new_stmt_iter.next() {
                new_body.push(assignment);
            }
        }
    }

    /// Replace import statements with assignments in the module body
    fn replace_import_statements_with_assignments(
        &self,
        module_ast: &mut ModModule,
        statements_to_replace: &[usize],
        new_statements: Vec<Stmt>,
    ) {
        let mut new_body = Vec::new();
        let mut new_stmt_iter = new_statements.into_iter();

        for (i, stmt) in module_ast.body.iter().enumerate() {
            if statements_to_replace.contains(&i) {
                // Replace this import statement with assignment(s)
                if let Stmt::ImportFrom(import_from_stmt) = stmt {
                    self.add_replacement_statements_for_import(
                        &mut new_body,
                        import_from_stmt,
                        &mut new_stmt_iter,
                    );
                }
            } else {
                new_body.push(stmt.clone());
            }
        }

        module_ast.body = new_body;
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::config::Config;
    fn create_test_emitter() -> CodeEmitter {
        let config = Config::default();
        let resolver =
            ModuleResolver::new(config).expect("ModuleResolver creation should succeed in test");
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
                .expect("filter_import_statements should succeed in test");

            // Should keep both import statements
            assert_eq!(filtered.len(), 2);
        }
    }

    #[test]
    fn test_filter_aliased_imports_logic() {
        use crate::ast_rewriter::AstRewriter;
        use indexmap::IndexSet;

        let emitter = create_test_emitter();

        // Create a real AstRewriter and populate it with import aliases using actual Python code
        let mut ast_rewriter = AstRewriter::new(10); // Python 3.10

        // Create a simulated Python module with import aliases
        let python_source = r#"
import numpy as np
import matplotlib.pyplot as plt
from pandas import DataFrame as pd
import os
"#;

        // Parse the Python source and collect import aliases
        let parsed =
            ruff_python_parser::parse_module(python_source).expect("Should parse test Python code");
        ast_rewriter.collect_import_aliases(parsed.syntax(), "test_module");

        // Set up import sets that contain our test modules
        let mut third_party_imports = IndexSet::from([
            "numpy".to_string(),
            "matplotlib.pyplot".to_string(),
            "pandas".to_string(),
        ]);
        let mut stdlib_imports = IndexSet::from(["os".to_string()]);

        // Store original state for comparison (currently unused but kept for future debugging)
        let _original_third_party = third_party_imports.clone();
        let _original_stdlib = stdlib_imports.clone();

        // Call the actual method we're testing
        let (aliased_third_party, aliased_stdlib) = emitter.filter_aliased_imports(
            &mut third_party_imports,
            &mut stdlib_imports,
            &ast_rewriter,
        );

        // Verify the correct behavior:
        // - numpy (has explicit alias "np", not from import) should be filtered to aliased_third_party
        // - matplotlib.pyplot (has explicit alias "plt", not from import) should be filtered to aliased_third_party
        // - pandas (has from import, should be skipped even though it has explicit alias)
        // - os (no explicit alias, should be skipped)

        // Should include numpy and matplotlib.pyplot (both have explicit aliases and are not from imports)
        assert_eq!(aliased_third_party.len(), 2);
        assert!(aliased_third_party.contains("numpy"));
        assert!(aliased_third_party.contains("matplotlib.pyplot"));

        // Should not include any stdlib imports (os doesn't have explicit alias)
        assert_eq!(aliased_stdlib.len(), 0);

        // pandas should remain in third_party_imports (is_from_import = true, so skipped)
        assert!(third_party_imports.contains("pandas"));

        // numpy and matplotlib.pyplot should be removed from third_party_imports
        assert!(!third_party_imports.contains("numpy"));
        assert!(!third_party_imports.contains("matplotlib.pyplot"));

        // os should remain in stdlib_imports (has_explicit_alias = false, so skipped)
        assert!(stdlib_imports.contains("os"));
    }
}
