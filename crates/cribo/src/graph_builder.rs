/// Graph builder that creates `CriboGraph` from Python AST
/// This module bridges the gap between ruff's AST and our dependency graph
use anyhow::Result;
use ruff_python_ast::{self as ast, Expr, ModModule, Stmt};

use crate::{
    cribo_graph::{ItemData, ItemType, ModuleDepGraph},
    types::{FxIndexMap, FxIndexSet},
    visitors::ExpressionSideEffectDetector,
};

/// Context for for statement variable collection
struct ForStmtContext<'a, 'b> {
    read_vars: &'a mut FxIndexSet<String>,
    write_vars: &'a mut FxIndexSet<String>,
    stack: &'a mut Vec<&'b [Stmt]>,
    attribute_accesses: &'a mut FxIndexMap<String, FxIndexSet<String>>,
}

/// Builds a `ModuleDepGraph` from a Python AST
pub struct GraphBuilder<'a> {
    graph: &'a mut ModuleDepGraph,
    current_scope: ScopeType,
    /// Track import aliases for importlib detection
    /// Maps local name -> module path (e.g., "il" -> "importlib", "im" ->
    /// "`importlib.import_module`")
    import_aliases: FxIndexMap<String, String>,
    python_version: u8,
    /// When inside a function or class, track the scope name
    scope_name: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum ScopeType {
    Module,
    Function,
    Class,
}

impl<'a> GraphBuilder<'a> {
    pub fn new(graph: &'a mut ModuleDepGraph, python_version: u8) -> Self {
        Self {
            graph,
            current_scope: ScopeType::Module,
            import_aliases: FxIndexMap::default(),
            python_version,
            scope_name: None,
        }
    }

    /// Build the graph from an AST
    pub fn build_from_ast(&mut self, ast: &ModModule) -> Result<()> {
        // Process all statements in the module
        log::trace!("Building graph from AST with {} statements", ast.body.len());
        for stmt in &ast.body {
            self.process_statement(stmt)?;
        }
        Ok(())
    }

    /// Process a statement and add it to the graph
    fn process_statement(&mut self, stmt: &Stmt) -> Result<()> {
        log::trace!(
            "process_statement: Processing statement type: {:?}",
            std::mem::discriminant(stmt)
        );
        // Inside functions, process imports, functions, and classes normally
        // Skip other statements as they're tracked via eventual_read_vars
        if matches!(self.current_scope, ScopeType::Function) {
            match stmt {
                Stmt::Import(import_stmt) => {
                    self.process_import(import_stmt);
                    return Ok(());
                }
                Stmt::ImportFrom(import_from) => {
                    self.process_import_from(import_from);
                    return Ok(());
                }
                Stmt::FunctionDef(func_def) => return self.process_function_def(func_def),
                Stmt::ClassDef(class_def) => return self.process_class_def(class_def),
                // Recurse into control flow blocks that may contain imports
                Stmt::If(_) | Stmt::For(_) | Stmt::While(_) | Stmt::With(_) | Stmt::Try(_) => {
                    // Fall through to regular processing to handle nested imports
                }
                _ => return Ok(()),
            }
        }

        match stmt {
            Stmt::Import(import_stmt) => {
                log::debug!("Processing import statement");
                self.process_import(import_stmt);
                Ok(())
            }
            Stmt::ImportFrom(import_from) => {
                self.process_import_from(import_from);
                Ok(())
            }
            Stmt::FunctionDef(func_def) => self.process_function_def(func_def),
            Stmt::ClassDef(class_def) => self.process_class_def(class_def),
            Stmt::Assign(assign) => {
                self.process_assign(assign);
                Ok(())
            }
            Stmt::AnnAssign(ann_assign) => {
                self.process_ann_assign(ann_assign);
                Ok(())
            }
            Stmt::Expr(expr_stmt) => {
                self.process_expr_stmt(&expr_stmt.value);
                Ok(())
            }
            Stmt::Assert(assert_stmt) => {
                self.process_assert_stmt(assert_stmt);
                Ok(())
            }
            Stmt::If(if_stmt) => self.process_if_stmt(if_stmt),
            Stmt::For(for_stmt) => self.process_for_stmt(for_stmt),
            Stmt::While(while_stmt) => self.process_while_stmt(while_stmt),
            Stmt::With(with_stmt) => self.process_with_stmt(with_stmt),
            Stmt::Try(try_stmt) => self.process_try_stmt(try_stmt),
            Stmt::Raise(raise_stmt) => {
                self.process_raise_stmt(raise_stmt);
                Ok(())
            }
            _ => Ok(()), // Other statements
        }
    }

    /// Process an import statement
    fn process_import(&mut self, import_stmt: &ast::StmtImport) {
        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();
            let local_name = alias
                .asname
                .as_ref()
                .map_or(module_name, ruff_python_ast::Identifier::as_str);

            log::trace!("Processing import: {module_name} as {local_name}");

            // Track importlib aliases for later detection
            if module_name == "importlib" {
                self.import_aliases
                    .insert(local_name.to_string(), "importlib".to_string());
            }

            let mut imported_names = FxIndexSet::default();
            let mut var_decls = FxIndexSet::default();

            // For imports like `import xml.etree.ElementTree`:
            // - The imported name is the full module path "xml.etree.ElementTree"
            // - The declared variable is determined by the alias or the module path
            if alias.asname.is_some() {
                // import xml.etree.ElementTree as ET
                // Imported: xml.etree.ElementTree, Declared: ET
                imported_names.insert(local_name.to_string());
                var_decls.insert(local_name.to_string());
            } else if module_name.contains('.') {
                // import xml.etree.ElementTree
                // Imported: xml.etree.ElementTree, Declared: xml (root variable)
                imported_names.insert(module_name.to_string());
                let root_module = module_name
                    .split('.')
                    .next()
                    .expect("module name should have at least one part");
                var_decls.insert(root_module.to_string());
            } else {
                // import os
                // Imported: os, Declared: os
                imported_names.insert(local_name.to_string());
                var_decls.insert(local_name.to_string());
            }

            let item_data = ItemData {
                item_type: ItemType::Import {
                    module: module_name.to_string(),
                    alias: alias.asname.as_ref().map(std::string::ToString::to_string),
                },
                var_decls,
                read_vars: FxIndexSet::default(),
                eventual_read_vars: FxIndexSet::default(),
                write_vars: FxIndexSet::default(),
                eventual_write_vars: FxIndexSet::default(),
                has_side_effects: crate::side_effects::import_has_side_effects(
                    module_name,
                    self.python_version,
                ),
                imported_names,
                reexported_names: FxIndexSet::default(),
                defined_symbols: FxIndexSet::default(),
                symbol_dependencies: FxIndexMap::default(),
                attribute_accesses: FxIndexMap::default(),
                containing_scope: self.scope_name.clone(),
            };

            self.graph.add_item(item_data);
        }
    }

    /// Process a from-import statement
    fn process_import_from(&mut self, import_from: &ast::StmtImportFrom) {
        let module_name = import_from
            .module
            .as_ref()
            .map_or("", ruff_python_ast::Identifier::as_str);

        // Skip __future__ imports as they're handled separately
        if module_name == "__future__" {
            return;
        }

        // For relative imports, we should not store the raw module name
        // It should be resolved to the full module path or marked as relative
        let effective_module = if import_from.level > 0 {
            // This is a relative import - mark it with dots
            let dots = ".".repeat(import_from.level as usize);
            if module_name.is_empty() {
                dots
            } else {
                format!("{dots}{module_name}")
            }
        } else {
            module_name.to_string()
        };

        let is_star = import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*";

        let mut imported_names = FxIndexSet::default();
        let mut names = Vec::new();
        let mut reexported_names = FxIndexSet::default();

        if is_star {
            imported_names.insert("*".to_string());
        } else {
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let local_name = alias
                    .asname
                    .as_ref()
                    .map_or(imported_name, ruff_python_ast::Identifier::as_str);

                imported_names.insert(local_name.to_string());
                names.push((
                    imported_name.to_string(),
                    alias.asname.as_ref().map(std::string::ToString::to_string),
                ));

                // Check for explicit re-export pattern: from foo import Bar as Bar
                if alias
                    .asname
                    .as_ref()
                    .map(ruff_python_ast::Identifier::as_str)
                    == Some(imported_name)
                {
                    reexported_names.insert(local_name.to_string());
                }

                // Track import_module from importlib
                if module_name == "importlib" && imported_name == "import_module" {
                    self.import_aliases.insert(
                        local_name.to_string(),
                        "importlib.import_module".to_string(),
                    );
                }
            }
        }

        let item_data = ItemData {
            item_type: ItemType::FromImport {
                module: effective_module.clone(),
                names,
                level: import_from.level,
                is_star,
            },
            var_decls: if is_star {
                FxIndexSet::default() // star-import declares nothing explicitly
            } else {
                imported_names.clone() // FromImport declares the imported names as variables
            },
            read_vars: FxIndexSet::default(),
            eventual_read_vars: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: crate::side_effects::from_import_has_side_effects(
                import_from,
                self.python_version,
            ),
            imported_names,
            reexported_names,
            defined_symbols: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);
    }

    /// Process a function definition
    fn process_function_def(&mut self, func_def: &ast::StmtFunctionDef) -> Result<()> {
        let func_name = func_def.name.to_string();

        // Collect variables from decorators and type annotations
        let mut read_vars = FxIndexSet::default();

        // Process decorators
        for decorator in &func_def.decorator_list {
            self.collect_vars_in_expr(&decorator.expression, &mut read_vars);
        }

        // Process parameter type annotations and defaults
        self.collect_function_parameter_vars(&func_def.parameters, &mut read_vars);

        // Process return type annotation
        if let Some(returns) = &func_def.returns {
            self.collect_vars_in_expr(returns, &mut read_vars);
        }

        // Collect variables that will be read within the function
        let mut eventual_read_vars = FxIndexSet::default();
        let mut eventual_write_vars = FxIndexSet::default();
        let mut eventual_attribute_accesses = FxIndexMap::default();
        self.collect_vars_in_body(
            &func_def.body,
            &mut eventual_read_vars,
            &mut eventual_write_vars,
            &mut eventual_attribute_accesses,
        );

        // Build symbol dependencies - the function depends on all variables it reads
        let mut symbol_dependencies = FxIndexMap::default();
        let mut all_deps = FxIndexSet::default();
        all_deps.extend(read_vars.clone());
        all_deps.extend(eventual_read_vars.clone());
        symbol_dependencies.insert(func_name.clone(), all_deps);

        log::debug!(
            "Function {func_name} has eventual_read_vars: {eventual_read_vars:?}, \
             eventual_write_vars: {eventual_write_vars:?}"
        );

        let item_data = ItemData {
            item_type: ItemType::FunctionDef {
                name: func_name.clone(),
            },
            var_decls: [func_name.clone()].into_iter().collect(),
            read_vars,
            eventual_read_vars,
            write_vars: FxIndexSet::default(),
            eventual_write_vars,
            has_side_effects: false,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: [func_name.clone()].into_iter().collect(),
            symbol_dependencies,
            attribute_accesses: eventual_attribute_accesses,
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);

        // Process the function body in function scope
        let old_scope = self.current_scope;
        let old_scope_name = self.scope_name.clone();
        self.current_scope = ScopeType::Function;
        self.scope_name = Some(func_name.clone());
        for stmt in &func_def.body {
            self.process_statement(stmt)?;
        }
        self.current_scope = old_scope;
        self.scope_name = old_scope_name;

        Ok(())
    }

    /// Process a class definition
    fn process_class_def(&mut self, class_def: &ast::StmtClassDef) -> Result<()> {
        let class_name = class_def.name.to_string();

        // Collect variables from decorators
        let mut read_vars = FxIndexSet::default();
        for decorator in &class_def.decorator_list {
            self.collect_vars_in_expr(&decorator.expression, &mut read_vars);
        }

        // Collect variables from base classes
        if let Some(_arguments) = &class_def.type_params {
            // Handle type parameters if present
            // Note: This is for generic classes
        }

        let mut attribute_accesses = FxIndexMap::default();
        if let Some(arguments) = &class_def.arguments {
            log::debug!(
                "Class {} has {} base classes",
                class_name,
                arguments.args.len()
            );
            for arg in &arguments.args {
                self.collect_vars_in_expr_with_attrs(arg, &mut read_vars, &mut attribute_accesses);
            }
            log::debug!("Class {class_name} base class read_vars: {read_vars:?}");
            // Collect variables from keyword arguments (e.g., metaclass=ABCMeta)
            for kw in &arguments.keywords {
                self.collect_vars_in_expr_with_attrs(
                    &kw.value,
                    &mut read_vars,
                    &mut attribute_accesses,
                );
            }
        }

        // Build symbol dependencies - the class depends on its base classes and decorators
        let mut symbol_dependencies = FxIndexMap::default();
        symbol_dependencies.insert(class_name.clone(), read_vars.clone());

        // Collect all variables used in methods to add as eventual dependencies
        let mut method_read_vars = FxIndexSet::default();
        let mut method_write_vars = FxIndexSet::default();
        let mut method_attribute_accesses = FxIndexMap::default();
        for stmt in &class_def.body {
            match stmt {
                Stmt::FunctionDef(method_def) => {
                    // Collect variables from method decorators
                    // While decorators execute at class definition time, they don't affect
                    // the ordering of the class itself - only the methods
                    for decorator in &method_def.decorator_list {
                        self.collect_vars_in_expr(&decorator.expression, &mut method_read_vars);
                    }

                    // Collect variables from method parameter annotations and defaults
                    // These don't affect class definition ordering
                    self.collect_function_parameter_vars(
                        &method_def.parameters,
                        &mut method_read_vars,
                    );

                    // Collect variables from return type annotation
                    if let Some(returns) = &method_def.returns {
                        self.collect_vars_in_expr(returns, &mut method_read_vars);
                    }

                    // Collect variables used in the method body (these are eventual dependencies)
                    self.collect_vars_in_body(
                        &method_def.body,
                        &mut method_read_vars,
                        &mut method_write_vars,
                        &mut method_attribute_accesses,
                    );
                }
                Stmt::Assign(assign) => {
                    // Class-level assignments like `yaml_loader = [Loader, FullLoader,
                    // UnsafeLoader]` These execute at class definition time, so
                    // they're immediate dependencies
                    self.collect_vars_in_expr(&assign.value, &mut read_vars);
                }
                Stmt::AnnAssign(ann_assign) => {
                    // Annotated class-level assignments (immediate deps)
                    // Read the annotation
                    self.collect_vars_in_expr_with_attrs(
                        &ann_assign.annotation,
                        &mut read_vars,
                        &mut method_attribute_accesses,
                    );
                    // Read the value if present
                    if let Some(value) = &ann_assign.value {
                        self.collect_vars_in_expr_with_attrs(
                            value,
                            &mut read_vars,
                            &mut method_attribute_accesses,
                        );
                    }
                    // Reads from attribute/subscript targets (e.g., cfg['x']: T = v)
                    self.collect_reads_from_assignment_target(&ann_assign.target, &mut read_vars);
                }
                _ => {
                    // Other statements in class body (e.g., docstrings)
                }
            }
        }

        // Merge attribute accesses from base classes and methods
        for (key, values) in attribute_accesses {
            method_attribute_accesses
                .entry(key)
                .or_default()
                .extend(values);
        }

        let item_data = ItemData {
            item_type: ItemType::ClassDef {
                name: class_name.clone(),
            },
            var_decls: [class_name.clone()].into_iter().collect(),
            read_vars,
            eventual_read_vars: method_read_vars, // Methods may use these variables
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: false,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: [class_name.clone()].into_iter().collect(),
            symbol_dependencies,
            attribute_accesses: method_attribute_accesses,
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);

        // Process the class body in class scope
        let old_scope = self.current_scope;
        let old_scope_name = self.scope_name.clone();
        self.current_scope = ScopeType::Class;
        self.scope_name = Some(class_name.clone());
        for stmt in &class_def.body {
            self.process_statement(stmt)?;
        }
        self.current_scope = old_scope;
        self.scope_name = old_scope_name;

        Ok(())
    }

    /// Process an assignment statement
    fn process_assign(&mut self, assign: &ast::StmtAssign) {
        let mut targets = Vec::new();
        let mut var_decls = FxIndexSet::default();

        for target in &assign.targets {
            if let Some(names) = self.extract_assignment_targets(target) {
                targets.extend(names.iter().cloned());
                var_decls.extend(names);
            }
        }

        // Collect variables read in the value expression
        let mut read_vars = FxIndexSet::default();
        let mut attribute_accesses = FxIndexMap::default();
        self.collect_vars_in_expr_with_attrs(
            &assign.value,
            &mut read_vars,
            &mut attribute_accesses,
        );

        if !attribute_accesses.is_empty() {
            log::debug!("Assignment collected attribute_accesses: {attribute_accesses:?}");
        }

        // Also collect reads from assignment targets (for subscript/attribute mutations)
        for target in &assign.targets {
            self.collect_reads_from_assignment_target(target, &mut read_vars);
        }

        // Check if this is an importlib.import_module() assignment
        if let Some(module_name) = self.is_static_importlib_call(&assign.value) {
            // This is an importlib.import_module() assignment
            // Track it as an import for tree-shaking purposes
            log::debug!(
                "Found importlib.import_module('{module_name}') assignment to: {targets:?}"
            );

            // Create an Import item for each target variable
            for target in &targets {
                let mut imported_names = FxIndexSet::default();
                imported_names.insert(module_name.clone());

                let item_data = ItemData {
                    item_type: ItemType::Import {
                        module: module_name.clone(),
                        alias: Some(target.clone()),
                    },
                    var_decls: [target.clone()].into_iter().collect(),
                    read_vars: read_vars.clone(),
                    eventual_read_vars: FxIndexSet::default(),
                    write_vars: FxIndexSet::default(),
                    eventual_write_vars: FxIndexSet::default(),
                    has_side_effects: true, // Import always has side effects
                    imported_names,
                    reexported_names: FxIndexSet::default(),
                    defined_symbols: [target.clone()].into_iter().collect(),
                    symbol_dependencies: FxIndexMap::default(),
                    attribute_accesses: FxIndexMap::default(),
                    containing_scope: self.scope_name.clone(),
                };

                self.graph.add_item(item_data);
            }
        } else {
            // Regular assignment
            // Check if this is an __all__ assignment
            let is_all_assignment = targets.contains(&"__all__".to_string());
            let mut reexported_names = FxIndexSet::default();

            if is_all_assignment {
                // Extract names from __all__ value
                if let Expr::List(list_expr) = assign.value.as_ref() {
                    reexported_names.extend(list_expr.elts.iter().filter_map(
                        |element| match element {
                            Expr::StringLiteral(string_lit) => Some(string_lit.value.to_string()),
                            _ => None,
                        },
                    ));
                }
            }

            // With the proxy approach, stdlib modules are handled dynamically,
            // so we treat all attribute accesses conservatively for side effects

            let item_data = ItemData {
                item_type: ItemType::Assignment {
                    targets: targets.clone(),
                },
                var_decls: var_decls.clone(),
                read_vars,
                eventual_read_vars: reexported_names.clone(), /* Names in __all__ are "eventually
                                                               * read" */
                write_vars: FxIndexSet::default(),
                eventual_write_vars: FxIndexSet::default(),
                has_side_effects: Self::expression_has_side_effects(&assign.value),
                imported_names: FxIndexSet::default(),
                reexported_names,
                defined_symbols: var_decls,
                symbol_dependencies: FxIndexMap::default(),
                attribute_accesses,
                containing_scope: self.scope_name.clone(),
            };

            self.graph.add_item(item_data);
        }
    }

    /// Process an annotated assignment statement
    fn process_ann_assign(&mut self, ann_assign: &ast::StmtAnnAssign) {
        let mut var_decls = FxIndexSet::default();
        let mut read_vars = FxIndexSet::default();

        // Extract target variable name
        if let Some(names) = self.extract_assignment_targets(&ann_assign.target) {
            var_decls.extend(names);
        }

        // Collect variables from the type annotation
        self.collect_vars_in_expr(&ann_assign.annotation, &mut read_vars);

        // Collect variables from the value expression if present
        if let Some(value) = &ann_assign.value {
            self.collect_vars_in_expr(value, &mut read_vars);
        }

        let item_data = ItemData {
            item_type: ItemType::Assignment {
                targets: var_decls.iter().cloned().collect(),
            },
            var_decls: var_decls.clone(),
            read_vars,
            eventual_read_vars: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: ann_assign
                .value
                .as_ref()
                .is_some_and(|v| Self::expression_has_side_effects(v)),
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: var_decls,
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);
    }

    /// Process an expression statement
    fn process_expr_stmt(&mut self, expr: &Expr) {
        let mut read_vars = FxIndexSet::default();
        let mut attribute_accesses = FxIndexMap::default();
        self.collect_vars_in_expr_with_attrs(expr, &mut read_vars, &mut attribute_accesses);

        log::debug!(
            "Processing expression statement, read_vars: {read_vars:?}, attribute_accesses: \
             {attribute_accesses:?}"
        );

        // Check if this is a docstring or other constant expression
        let has_side_effects = match expr {
            // Docstrings and constant expressions don't have side effects
            Expr::StringLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::EllipsisLiteral(_) => false,
            // For other expressions, check using the side effect detector
            _ => Self::expression_has_side_effects(expr),
        };

        let item_data = ItemData {
            item_type: ItemType::Expression,
            var_decls: FxIndexSet::default(),
            read_vars,
            eventual_read_vars: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses,
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);
    }

    /// Process assert statement
    fn process_assert_stmt(&mut self, assert_stmt: &ast::StmtAssert) {
        let mut read_vars = FxIndexSet::default();
        let mut attribute_accesses = FxIndexMap::default();

        // Collect variables from the test expression
        self.collect_vars_in_expr_with_attrs(
            &assert_stmt.test,
            &mut read_vars,
            &mut attribute_accesses,
        );

        // Also collect from the message expression if present
        if let Some(msg) = &assert_stmt.msg {
            self.collect_vars_in_expr_with_attrs(msg, &mut read_vars, &mut attribute_accesses);
        }

        log::debug!(
            "Processing assert statement, read_vars: {read_vars:?}, attribute_accesses: \
             {attribute_accesses:?}"
        );

        let item_data = ItemData {
            item_type: ItemType::Expression, // Assert is treated as an expression with side effects
            var_decls: FxIndexSet::default(),
            read_vars,
            eventual_read_vars: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: true, /* Assert statements have side effects (can raise
                                     * AssertionError) */
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses,
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);
    }

    /// Process if statement
    fn process_if_stmt(&mut self, if_stmt: &ast::StmtIf) -> Result<()> {
        // Process condition
        let mut read_vars = FxIndexSet::default();
        self.collect_vars_in_expr(&if_stmt.test, &mut read_vars);

        let item_data = ItemData {
            item_type: ItemType::If {
                condition: String::new(), // Could extract condition text if needed
            },
            var_decls: FxIndexSet::default(),
            read_vars,
            eventual_read_vars: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: true,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);

        // Process body
        for stmt in &if_stmt.body {
            self.process_statement(stmt)?;
        }

        // Process elif/else branches
        for clause in &if_stmt.elif_else_clauses {
            if let Some(condition) = &clause.test {
                let mut read_vars = FxIndexSet::default();
                self.collect_vars_in_expr(condition, &mut read_vars);
                // Could add as separate If item
            }
            for stmt in &clause.body {
                self.process_statement(stmt)?;
            }
        }

        Ok(())
    }

    /// Process for loop
    fn process_for_stmt(&mut self, for_stmt: &ast::StmtFor) -> Result<()> {
        let mut read_vars = FxIndexSet::default();
        self.collect_vars_in_expr(&for_stmt.iter, &mut read_vars);

        // Extract loop variables
        let mut write_vars = FxIndexSet::default();
        if let Some(names) = self.extract_assignment_targets(&for_stmt.target) {
            write_vars.extend(names);
        }

        let item_data = ItemData {
            item_type: ItemType::Other,
            var_decls: FxIndexSet::default(),
            read_vars,
            eventual_read_vars: FxIndexSet::default(),
            write_vars,
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: true,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);

        // Process body
        for stmt in &for_stmt.body {
            self.process_statement(stmt)?;
        }

        // Process else clause
        for stmt in &for_stmt.orelse {
            self.process_statement(stmt)?;
        }

        Ok(())
    }

    /// Process while loop
    fn process_while_stmt(&mut self, while_stmt: &ast::StmtWhile) -> Result<()> {
        let mut read_vars = FxIndexSet::default();
        self.collect_vars_in_expr(&while_stmt.test, &mut read_vars);

        let item_data = ItemData {
            item_type: ItemType::Other,
            var_decls: FxIndexSet::default(),
            read_vars,
            eventual_read_vars: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: true,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);

        // Process body
        for stmt in &while_stmt.body {
            self.process_statement(stmt)?;
        }

        // Process else clause
        for stmt in &while_stmt.orelse {
            self.process_statement(stmt)?;
        }

        Ok(())
    }

    /// Process with statement
    fn process_with_stmt(&mut self, with_stmt: &ast::StmtWith) -> Result<()> {
        let mut read_vars = FxIndexSet::default();

        for item in &with_stmt.items {
            self.collect_vars_in_expr(&item.context_expr, &mut read_vars);
        }

        let item_data = ItemData {
            item_type: ItemType::Other,
            var_decls: FxIndexSet::default(),
            read_vars,
            eventual_read_vars: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: true,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);

        // Process body
        for stmt in &with_stmt.body {
            self.process_statement(stmt)?;
        }

        Ok(())
    }

    /// Process raise statement
    fn process_raise_stmt(&mut self, raise_stmt: &ast::StmtRaise) {
        log::debug!("Processing raise statement");

        let mut read_vars = FxIndexSet::default();
        let mut attribute_accesses = FxIndexMap::default();

        // Collect variables from the exception expression
        if let Some(exc) = &raise_stmt.exc {
            self.collect_vars_in_expr_with_attrs(exc, &mut read_vars, &mut attribute_accesses);
        }

        // Also collect from the cause expression if present
        if let Some(cause) = &raise_stmt.cause {
            self.collect_vars_in_expr_with_attrs(cause, &mut read_vars, &mut attribute_accesses);
        }

        log::debug!(
            "Processing raise statement, read_vars: {read_vars:?}, attribute_accesses: \
             {attribute_accesses:?}"
        );

        let item_data = ItemData {
            item_type: ItemType::Other,
            var_decls: FxIndexSet::default(),
            read_vars,
            eventual_read_vars: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: true, // Raise statements have side effects
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses,
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);
    }

    /// Process try statement
    fn process_try_stmt(&mut self, try_stmt: &ast::StmtTry) -> Result<()> {
        log::debug!(
            "Processing try statement with {} statements in body",
            try_stmt.body.len()
        );

        let item_data = ItemData {
            item_type: ItemType::Try,
            var_decls: FxIndexSet::default(),
            read_vars: FxIndexSet::default(),
            eventual_read_vars: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: true,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: self.scope_name.clone(),
        };

        self.graph.add_item(item_data);

        // Process try body
        for stmt in &try_stmt.body {
            self.process_statement(stmt)?;
        }

        // Process except handlers
        for handler in &try_stmt.handlers {
            let ast::ExceptHandler::ExceptHandler(handler) = handler;

            // Track exception type if specified
            if let Some(type_expr) = &handler.type_ {
                let mut read_vars = FxIndexSet::default();
                let mut attribute_accesses = FxIndexMap::default();
                self.collect_vars_in_expr_with_attrs(
                    type_expr,
                    &mut read_vars,
                    &mut attribute_accesses,
                );

                // Create an item for the exception handler
                let item_data = ItemData {
                    item_type: ItemType::Other,
                    var_decls: FxIndexSet::default(),
                    read_vars,
                    eventual_read_vars: FxIndexSet::default(),
                    write_vars: FxIndexSet::default(),
                    eventual_write_vars: FxIndexSet::default(),
                    has_side_effects: false,
                    imported_names: FxIndexSet::default(),
                    reexported_names: FxIndexSet::default(),
                    defined_symbols: FxIndexSet::default(),
                    symbol_dependencies: FxIndexMap::default(),
                    attribute_accesses,
                    containing_scope: self.scope_name.clone(),
                };
                self.graph.add_item(item_data);
            }

            for stmt in &handler.body {
                self.process_statement(stmt)?;
            }
        }

        // Process else clause
        for stmt in &try_stmt.orelse {
            self.process_statement(stmt)?;
        }

        // Process finally clause
        for stmt in &try_stmt.finalbody {
            self.process_statement(stmt)?;
        }

        Ok(())
    }

    /// Extract assignment target names
    fn extract_assignment_targets(&self, expr: &Expr) -> Option<Vec<String>> {
        let mut names = Vec::new();
        let mut stack = vec![expr];

        while let Some(current_expr) = stack.pop() {
            match current_expr {
                Expr::Name(name) => {
                    names.push(name.id.to_string());
                }
                Expr::Tuple(tuple) => {
                    stack.extend(tuple.elts.iter());
                }
                Expr::List(list) => {
                    stack.extend(list.elts.iter());
                }
                Expr::Subscript(_) | Expr::Attribute(_) => {
                    // For subscript (e.g., result["key"]) and attribute (e.g., obj.attr)
                    // assignments, we don't add them to write_vars as they
                    // don't create new variables However, we need to track that
                    // they're being mutated - handled separately
                }
                _ => return None, // Unsupported target type
            }
        }

        if names.is_empty() { None } else { Some(names) }
    }

    /// Collect variables used in an expression and track attribute accesses
    fn collect_vars_in_expr_with_attrs(
        &self,
        expr: &Expr,
        vars: &mut FxIndexSet<String>,
        attribute_accesses: &mut FxIndexMap<String, FxIndexSet<String>>,
    ) {
        match expr {
            Expr::Name(name) => {
                vars.insert(name.id.to_string());
            }
            Expr::Attribute(attr) => {
                // Track attribute access for tree-shaking
                if let Expr::Name(base_name) = attr.value.as_ref() {
                    // Direct attribute access like greetings.message
                    let base = base_name.id.to_string();
                    vars.insert(base.clone());

                    // Track that we're accessing 'message' on 'greetings'
                    attribute_accesses
                        .entry(base)
                        .or_default()
                        .insert(attr.attr.to_string());
                } else if let Expr::Attribute(_base_attr) = attr.value.as_ref() {
                    // Nested attribute access like greetings.greeting.get_greeting
                    // For nested attributes, we need to build the full dotted path of attr.value
                    // So for greetings.greeting.get_greeting, attr.value is greetings.greeting
                    // and we want to track that we're accessing 'get_greeting' on
                    // 'greetings.greeting'

                    // Build the full dotted name from attr.value
                    fn build_full_dotted_name(expr: &Expr) -> Option<String> {
                        match expr {
                            Expr::Name(name) => Some(name.id.to_string()),
                            Expr::Attribute(attr) => build_full_dotted_name(&attr.value)
                                .map(|base| format!("{}.{}", base, attr.attr)),
                            _ => None,
                        }
                    }

                    if let Some(base_path) = build_full_dotted_name(attr.value.as_ref()) {
                        log::debug!(
                            "Nested attribute access: base_path='{}', attr='{}'",
                            base_path,
                            attr.attr
                        );
                        // Track that we're accessing 'get_greeting' on 'greetings.greeting'
                        attribute_accesses
                            .entry(base_path.clone())
                            .or_default()
                            .insert(attr.attr.to_string());

                        // Also track the base path as a read variable
                        vars.insert(base_path);
                    }
                }

                // Collect the base object, especially important for module attribute access
                // like `simple_module.__all__` or `xml.etree.ElementTree.__name__`

                // First, try to collect the full dotted name for module access
                if let Some(full_name) = self.extract_dotted_name(attr) {
                    // For dotted names like xml.etree.ElementTree, we need to check
                    // if this matches any imported module names
                    vars.insert(full_name.clone());

                    // Also add the root module name for compatibility
                    if full_name.contains('.') {
                        let root = full_name
                            .split('.')
                            .next()
                            .expect("full_name should have at least one part");
                        vars.insert(root.to_string());
                    }
                }

                // Also do the standard recursive collection
                match attr.value.as_ref() {
                    Expr::Name(name) => {
                        // Direct attribute access on a name (e.g., module.__all__)
                        vars.insert(name.id.to_string());
                    }
                    Expr::Attribute(_) => {
                        // For nested attributes, recursively collect vars
                        self.collect_vars_in_expr_with_attrs(&attr.value, vars, attribute_accesses);
                    }
                    _ => {
                        // For other types, recursively collect vars
                        self.collect_vars_in_expr_with_attrs(&attr.value, vars, attribute_accesses);
                    }
                }
            }
            Expr::Call(call) => {
                self.collect_vars_in_expr_with_attrs(&call.func, vars, attribute_accesses);
                for arg in &call.arguments.args {
                    self.collect_vars_in_expr_with_attrs(arg, vars, attribute_accesses);
                }
                for keyword in &call.arguments.keywords {
                    self.collect_vars_in_expr_with_attrs(&keyword.value, vars, attribute_accesses);
                }
            }
            Expr::BinOp(binop) => {
                self.collect_vars_in_expr_with_attrs(&binop.left, vars, attribute_accesses);
                self.collect_vars_in_expr_with_attrs(&binop.right, vars, attribute_accesses);
            }
            Expr::UnaryOp(unaryop) => {
                self.collect_vars_in_expr_with_attrs(&unaryop.operand, vars, attribute_accesses);
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.collect_vars_in_expr_with_attrs(elt, vars, attribute_accesses);
                }
            }
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.collect_vars_in_expr_with_attrs(elt, vars, attribute_accesses);
                }
            }
            Expr::Dict(dict) => {
                for item in &dict.items {
                    if let Some(key) = &item.key {
                        self.collect_vars_in_expr_with_attrs(key, vars, attribute_accesses);
                    }
                    self.collect_vars_in_expr_with_attrs(&item.value, vars, attribute_accesses);
                }
            }
            Expr::Set(set) => {
                for elt in &set.elts {
                    self.collect_vars_in_expr_with_attrs(elt, vars, attribute_accesses);
                }
            }
            Expr::Subscript(subscript) => {
                self.collect_vars_in_expr_with_attrs(&subscript.value, vars, attribute_accesses);
                self.collect_vars_in_expr_with_attrs(&subscript.slice, vars, attribute_accesses);
            }
            Expr::Compare(compare) => {
                self.collect_vars_in_expr_with_attrs(&compare.left, vars, attribute_accesses);
                for comparator in &compare.comparators {
                    self.collect_vars_in_expr_with_attrs(comparator, vars, attribute_accesses);
                }
            }
            Expr::BoolOp(boolop) => {
                for value in &boolop.values {
                    self.collect_vars_in_expr_with_attrs(value, vars, attribute_accesses);
                }
            }
            Expr::If(ifexp) => {
                self.collect_vars_in_expr_with_attrs(&ifexp.test, vars, attribute_accesses);
                self.collect_vars_in_expr_with_attrs(&ifexp.body, vars, attribute_accesses);
                self.collect_vars_in_expr_with_attrs(&ifexp.orelse, vars, attribute_accesses);
            }
            Expr::ListComp(comp) => {
                self.collect_vars_in_expr_with_attrs(&comp.elt, vars, attribute_accesses);
                for generator in &comp.generators {
                    self.collect_vars_in_expr_with_attrs(&generator.iter, vars, attribute_accesses);
                    for if_clause in &generator.ifs {
                        self.collect_vars_in_expr_with_attrs(if_clause, vars, attribute_accesses);
                    }
                }
            }
            Expr::SetComp(comp) => {
                self.collect_vars_in_expr_with_attrs(&comp.elt, vars, attribute_accesses);
                for generator in &comp.generators {
                    self.collect_vars_in_expr_with_attrs(&generator.iter, vars, attribute_accesses);
                    for if_clause in &generator.ifs {
                        self.collect_vars_in_expr_with_attrs(if_clause, vars, attribute_accesses);
                    }
                }
            }
            Expr::Generator(comp) => {
                self.collect_vars_in_expr_with_attrs(&comp.elt, vars, attribute_accesses);
                for generator in &comp.generators {
                    self.collect_vars_in_expr_with_attrs(&generator.iter, vars, attribute_accesses);
                    for if_clause in &generator.ifs {
                        self.collect_vars_in_expr_with_attrs(if_clause, vars, attribute_accesses);
                    }
                }
            }
            Expr::DictComp(comp) => {
                self.collect_vars_in_expr_with_attrs(&comp.key, vars, attribute_accesses);
                self.collect_vars_in_expr_with_attrs(&comp.value, vars, attribute_accesses);
                for generator in &comp.generators {
                    self.collect_vars_in_expr_with_attrs(&generator.iter, vars, attribute_accesses);
                    for if_clause in &generator.ifs {
                        self.collect_vars_in_expr_with_attrs(if_clause, vars, attribute_accesses);
                    }
                }
            }
            Expr::FString(fstring) => {
                // Process f-string value parts
                for element in fstring.value.elements() {
                    if let ast::InterpolatedStringElement::Interpolation(expr_element) = element {
                        self.collect_vars_in_expr_with_attrs(
                            &expr_element.expression,
                            vars,
                            attribute_accesses,
                        );
                    }
                }
            }
            _ => {} // Literals and other non-variable expressions
        }
    }

    /// Collect variables used in an expression
    fn collect_vars_in_expr(&self, expr: &Expr, vars: &mut FxIndexSet<String>) {
        // Use the new method but ignore attribute accesses for backward compatibility
        let mut dummy_attrs = FxIndexMap::default();
        self.collect_vars_in_expr_with_attrs(expr, vars, &mut dummy_attrs);
    }

    /// Collect variables in a statement body
    fn collect_vars_in_body(
        &self,
        body: &[Stmt],
        read_vars: &mut FxIndexSet<String>,
        write_vars: &mut FxIndexSet<String>,
        attribute_accesses: &mut FxIndexMap<String, FxIndexSet<String>>,
    ) {
        let mut stack: Vec<&[Stmt]> = vec![body];

        while let Some(current_body) = stack.pop() {
            for stmt in current_body {
                match stmt {
                    Stmt::Expr(expr_stmt) => {
                        self.collect_vars_in_expr_with_attrs(
                            &expr_stmt.value,
                            read_vars,
                            attribute_accesses,
                        );
                    }
                    Stmt::Assign(assign) => {
                        self.collect_vars_in_expr_with_attrs(
                            &assign.value,
                            read_vars,
                            attribute_accesses,
                        );
                        // Handle assignment targets to collect reads from subscripts/attributes
                        let mut dummy_write_vars = FxIndexSet::default();
                        self.handle_assign_targets(
                            &assign.targets,
                            &mut dummy_write_vars,
                            read_vars,
                        );
                        // Also add actual write targets
                        for target in &assign.targets {
                            if let Some(names) = self.extract_assignment_targets(target) {
                                write_vars.extend(names);
                            }
                        }
                    }
                    Stmt::Return(ret) => {
                        self.handle_return_stmt(ret, read_vars, attribute_accesses);
                    }
                    Stmt::If(if_stmt) => {
                        self.handle_if_stmt(if_stmt, read_vars, &mut stack, attribute_accesses);
                    }
                    Stmt::For(for_stmt) => {
                        let mut ctx = ForStmtContext {
                            read_vars,
                            write_vars,
                            stack: &mut stack,
                            attribute_accesses,
                        };
                        self.handle_for_stmt(for_stmt, &mut ctx);
                    }
                    Stmt::While(while_stmt) => {
                        self.collect_vars_in_expr_with_attrs(
                            &while_stmt.test,
                            read_vars,
                            attribute_accesses,
                        );
                        stack.push(&while_stmt.body);
                        stack.push(&while_stmt.orelse);
                    }
                    Stmt::With(with_stmt) => {
                        self.handle_with_stmt(with_stmt, read_vars, &mut stack, attribute_accesses);
                    }
                    Stmt::Try(try_stmt) => {
                        // Process the try body
                        stack.push(&try_stmt.body);
                        // Process exception handlers
                        for handler in &try_stmt.handlers {
                            match handler {
                                ast::ExceptHandler::ExceptHandler(except_handler) => {
                                    // Process the test expression if present
                                    if let Some(test_expr) = &except_handler.type_ {
                                        self.collect_vars_in_expr_with_attrs(
                                            test_expr,
                                            read_vars,
                                            attribute_accesses,
                                        );
                                    }
                                    // Process the handler body
                                    stack.push(&except_handler.body);
                                }
                            }
                        }
                        // Process else clause
                        stack.push(&try_stmt.orelse);
                        // Process finally clause
                        stack.push(&try_stmt.finalbody);
                    }
                    Stmt::Global(global_stmt) => {
                        // Global statements indicate that the function will read/write global
                        // variables
                        for name in &global_stmt.names {
                            // Add to both read_vars and write_vars since global vars can be both
                            // read and written
                            log::debug!("Found global statement for variable: {name}");
                            read_vars.insert(name.to_string());
                            write_vars.insert(name.to_string());
                        }
                    }
                    Stmt::Import(import_stmt) => {
                        // Track imported modules as read variables
                        for alias in &import_stmt.names {
                            let local_name = alias
                                .asname
                                .as_ref()
                                .map_or(alias.name.as_str(), ruff_python_ast::Identifier::as_str);
                            read_vars.insert(local_name.to_string());
                        }
                    }
                    Stmt::ImportFrom(import_from) => {
                        // Track imported names as read variables
                        if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*"
                        {
                            // Star imports are complex, skip for now
                        } else {
                            for alias in &import_from.names {
                                let local_name = alias.asname.as_ref().map_or(
                                    alias.name.as_str(),
                                    ruff_python_ast::Identifier::as_str,
                                );
                                read_vars.insert(local_name.to_string());
                            }
                        }
                    }
                    Stmt::Assert(assert_stmt) => {
                        self.collect_vars_in_expr_with_attrs(
                            &assert_stmt.test,
                            read_vars,
                            attribute_accesses,
                        );
                        if let Some(msg) = &assert_stmt.msg {
                            self.collect_vars_in_expr_with_attrs(
                                msg,
                                read_vars,
                                attribute_accesses,
                            );
                        }
                    }
                    Stmt::Raise(raise_stmt) => {
                        // Collect variables from raise statement
                        if let Some(exc) = &raise_stmt.exc {
                            self.collect_vars_in_expr_with_attrs(
                                exc,
                                read_vars,
                                attribute_accesses,
                            );
                        }
                        if let Some(cause) = &raise_stmt.cause {
                            self.collect_vars_in_expr_with_attrs(
                                cause,
                                read_vars,
                                attribute_accesses,
                            );
                        }
                    }
                    Stmt::ClassDef(class_def) => {
                        // Collect variables from decorators
                        for decorator in &class_def.decorator_list {
                            self.collect_vars_in_expr_with_attrs(
                                &decorator.expression,
                                read_vars,
                                attribute_accesses,
                            );
                        }

                        // Collect variables from base classes
                        if let Some(arguments) = &class_def.arguments {
                            for arg in &arguments.args {
                                self.collect_vars_in_expr_with_attrs(
                                    arg,
                                    read_vars,
                                    attribute_accesses,
                                );
                            }
                            // Collect variables from keyword arguments (e.g., metaclass=ABCMeta)
                            for kw in &arguments.keywords {
                                self.collect_vars_in_expr_with_attrs(
                                    &kw.value,
                                    read_vars,
                                    attribute_accesses,
                                );
                            }
                        }

                        // Recursively process the class body
                        stack.push(&class_def.body);
                    }
                    Stmt::FunctionDef(func_def) => {
                        // Collect variables from decorators
                        for decorator in &func_def.decorator_list {
                            self.collect_vars_in_expr_with_attrs(
                                &decorator.expression,
                                read_vars,
                                attribute_accesses,
                            );
                        }

                        // Collect from parameter annotations and defaults
                        self.collect_function_parameter_vars_with_attrs(
                            &func_def.parameters,
                            read_vars,
                            attribute_accesses,
                        );

                        // Collect from return type annotation
                        if let Some(returns) = &func_def.returns {
                            self.collect_vars_in_expr_with_attrs(
                                returns,
                                read_vars,
                                attribute_accesses,
                            );
                        }

                        // Recursively process the function body
                        stack.push(&func_def.body);
                    }
                    _ => {} // Other statements
                }
            }
        }
    }

    /// Check if an expression has side effects
    fn expression_has_side_effects(expr: &Expr) -> bool {
        // Delegates to visitor-based detector
        ExpressionSideEffectDetector::check(expr)
    }

    /// Extract a dotted name from an attribute expression
    /// e.g., xml.etree.ElementTree.__name__ -> Some("xml.etree.ElementTree")
    fn extract_dotted_name(&self, attr: &ast::ExprAttribute) -> Option<String> {
        // We want to extract the dotted name up to but not including the final attribute
        // For example: xml.etree.ElementTree.__name__ -> "xml.etree.ElementTree"

        fn build_dotted_name(expr: &Expr, parts: &mut Vec<String>) -> bool {
            match expr {
                Expr::Name(name) => {
                    parts.push(name.id.to_string());
                    true
                }
                Expr::Attribute(attr) => {
                    if build_dotted_name(&attr.value, parts) {
                        parts.push(attr.attr.to_string());
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            }
        }

        let mut parts = Vec::new();
        if build_dotted_name(&attr.value, &mut parts) {
            // Reverse because we built it bottom-up
            parts.reverse();
            Some(parts.join("."))
        } else {
            None
        }
    }

    /// Handle return statement variable collection
    fn handle_return_stmt(
        &self,
        ret: &ast::StmtReturn,
        read_vars: &mut FxIndexSet<String>,
        attribute_accesses: &mut FxIndexMap<String, FxIndexSet<String>>,
    ) {
        if let Some(value) = &ret.value {
            self.collect_vars_in_expr_with_attrs(value, read_vars, attribute_accesses);
        }
    }

    /// Handle assignment targets
    fn handle_assign_targets(
        &self,
        targets: &[Expr],
        write_vars: &mut FxIndexSet<String>,
        read_vars: &mut FxIndexSet<String>,
    ) {
        for target in targets {
            // First extract simple assignment targets (variable names)
            if let Some(names) = self.extract_assignment_targets(target) {
                write_vars.extend(names);
            }

            // Additionally, for subscript and attribute assignments, we need to track reads
            self.collect_reads_from_assignment_target(target, read_vars);
        }
    }

    /// Collect variables that are read when assigning to subscripts or attributes
    fn collect_reads_from_assignment_target(
        &self,
        target: &Expr,
        read_vars: &mut FxIndexSet<String>,
    ) {
        match target {
            Expr::Subscript(subscript) => {
                // For result["key"] = value, we're reading 'result' to mutate it
                log::debug!("Found subscript assignment target, collecting reads from base object");
                self.collect_vars_in_expr(&subscript.value, read_vars);
            }
            Expr::Attribute(attr) => {
                // For obj.attr = value, we're reading 'obj' to mutate it
                self.collect_vars_in_expr(&attr.value, read_vars);
            }
            Expr::Tuple(tuple) => {
                // Handle tuple unpacking which might contain subscripts/attributes
                for elt in &tuple.elts {
                    self.collect_reads_from_assignment_target(elt, read_vars);
                }
            }
            Expr::List(list) => {
                // Handle list unpacking which might contain subscripts/attributes
                for elt in &list.elts {
                    self.collect_reads_from_assignment_target(elt, read_vars);
                }
            }
            _ => {
                // Simple names don't need special handling here
            }
        }
    }

    /// Handle if statement variable collection
    fn handle_if_stmt<'b>(
        &self,
        if_stmt: &'b ast::StmtIf,
        read_vars: &mut FxIndexSet<String>,
        stack: &mut Vec<&'b [Stmt]>,
        attribute_accesses: &mut FxIndexMap<String, FxIndexSet<String>>,
    ) {
        self.collect_vars_in_expr_with_attrs(&if_stmt.test, read_vars, attribute_accesses);
        stack.push(&if_stmt.body);
        for clause in &if_stmt.elif_else_clauses {
            if let Some(condition) = &clause.test {
                self.collect_vars_in_expr_with_attrs(condition, read_vars, attribute_accesses);
            }
            stack.push(&clause.body);
        }
    }

    /// Handle for statement variable collection
    fn handle_for_stmt<'b>(&self, for_stmt: &'b ast::StmtFor, ctx: &mut ForStmtContext<'_, 'b>) {
        self.collect_vars_in_expr_with_attrs(&for_stmt.iter, ctx.read_vars, ctx.attribute_accesses);
        if let Some(names) = self.extract_assignment_targets(&for_stmt.target) {
            ctx.write_vars.extend(names);
        }
        ctx.stack.push(&for_stmt.body);
        ctx.stack.push(&for_stmt.orelse);
    }

    /// Check if an expression is an `importlib.import_module()` call with a static string argument
    fn is_static_importlib_call(&self, expr: &Expr) -> Option<String> {
        if let Expr::Call(call) = expr {
            // Check if this is importlib.import_module() or an alias
            let is_import_module = match &*call.func {
                // Direct call: importlib.import_module() or alias.import_module()
                Expr::Attribute(attr) if attr.attr.as_str() == "import_module" => {
                    if let Expr::Name(name) = &*attr.value {
                        let name_str = name.id.as_str();
                        // Check if it's importlib directly or an alias
                        name_str == "importlib"
                            || self
                                .import_aliases
                                .get(name_str)
                                .is_some_and(|v| v == "importlib")
                    } else {
                        false
                    }
                }
                // Direct function call: import_module() or im()
                Expr::Name(name) => {
                    let name_str = name.id.as_str();
                    // Check if this is import_module or an alias for it
                    name_str == "import_module"
                        || self
                            .import_aliases
                            .get(name_str)
                            .is_some_and(|v| v == "importlib.import_module")
                }
                _ => false,
            };

            if is_import_module {
                // Extract the module name if it's a static string
                if let Some(arg) = call.arguments.args.first()
                    && let Expr::StringLiteral(string_lit) = arg
                {
                    return Some(string_lit.value.to_string());
                }
            }
        }
        None
    }

    /// Handle with statement variable collection
    fn handle_with_stmt<'b>(
        &self,
        with_stmt: &'b ast::StmtWith,
        read_vars: &mut FxIndexSet<String>,
        stack: &mut Vec<&'b [Stmt]>,
        attribute_accesses: &mut FxIndexMap<String, FxIndexSet<String>>,
    ) {
        for item in &with_stmt.items {
            self.collect_vars_in_expr_with_attrs(&item.context_expr, read_vars, attribute_accesses);
        }
        stack.push(&with_stmt.body);
    }

    /// Collect variables from function parameters (annotations and defaults)
    /// This helper reduces duplication between function, method, and nested function processing
    fn collect_function_parameter_vars(
        &self,
        parameters: &ast::Parameters,
        vars: &mut FxIndexSet<String>,
    ) {
        // Process parameter type annotations and defaults
        for param in parameters
            .posonlyargs
            .iter()
            .chain(parameters.args.iter())
            .chain(parameters.kwonlyargs.iter())
        {
            if let Some(annotation) = &param.parameter.annotation {
                self.collect_vars_in_expr(annotation, vars);
            }
            if let Some(default) = &param.default {
                self.collect_vars_in_expr(default, vars);
            }
        }

        // Process vararg annotation
        if let Some(vararg) = &parameters.vararg
            && let Some(annotation) = &vararg.annotation
        {
            self.collect_vars_in_expr(annotation, vars);
        }

        // Process kwarg annotation
        if let Some(kwarg) = &parameters.kwarg
            && let Some(annotation) = &kwarg.annotation
        {
            self.collect_vars_in_expr(annotation, vars);
        }
    }

    /// Collect variables from function parameters with attribute tracking
    /// This variant tracks attribute accesses for more detailed analysis
    fn collect_function_parameter_vars_with_attrs(
        &self,
        parameters: &ast::Parameters,
        vars: &mut FxIndexSet<String>,
        attribute_accesses: &mut FxIndexMap<String, FxIndexSet<String>>,
    ) {
        // Process parameter type annotations and defaults
        for param in parameters
            .posonlyargs
            .iter()
            .chain(parameters.args.iter())
            .chain(parameters.kwonlyargs.iter())
        {
            if let Some(annotation) = &param.parameter.annotation {
                self.collect_vars_in_expr_with_attrs(annotation, vars, attribute_accesses);
            }
            if let Some(default) = &param.default {
                self.collect_vars_in_expr_with_attrs(default, vars, attribute_accesses);
            }
        }

        // Process vararg annotation
        if let Some(vararg) = &parameters.vararg
            && let Some(annotation) = &vararg.annotation
        {
            self.collect_vars_in_expr_with_attrs(annotation, vars, attribute_accesses);
        }

        // Process kwarg annotation
        if let Some(kwarg) = &parameters.kwarg
            && let Some(annotation) = &kwarg.annotation
        {
            self.collect_vars_in_expr_with_attrs(annotation, vars, attribute_accesses);
        }
    }
}
