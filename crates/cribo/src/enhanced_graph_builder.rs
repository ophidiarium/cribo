/// Enhanced graph builder that tracks import usage context
/// This module extends the basic graph builder to track where imports are used
use anyhow::Result;
use log::debug;
use ruff_python_ast::{self as ast, Expr, ModModule, Stmt};
use rustc_hash::FxHashSet;

use crate::cribo_graph::{ItemData, ItemId, ItemType, ModuleDepGraph};

/// Context tracking for import usage
#[derive(Debug, Clone)]
pub struct ImportUsageContext {
    /// Import statement item ID
    pub import_item_id: ItemId,
    /// Imported name
    pub imported_name: String,
    /// Items that use this import at module level
    pub module_level_usage: Vec<ItemId>,
    /// Items that use this import inside functions
    pub function_level_usage: Vec<ItemId>,
    /// Items that use this import inside classes
    pub class_level_usage: Vec<ItemId>,
}

/// Enhanced graph builder that tracks detailed import usage
pub struct EnhancedGraphBuilder<'a> {
    graph: &'a mut ModuleDepGraph,
    current_scope: ScopeType,
    /// Stack of scopes for tracking context
    scope_stack: Vec<ScopeContext>,
    /// Import usage tracking
    import_usage: FxHashSet<ImportUsageContext>,
}

#[derive(Debug, Clone)]
enum ScopeType {
    Module,
    Function(String), // Function name
    Class(String),    // Class name
}

#[derive(Debug, Clone)]
struct ScopeContext {
    scope_type: ScopeType,
    /// Variables that should be tracked as "eventual" reads in this scope
    deferred_reads: FxHashSet<String>,
}

impl<'a> EnhancedGraphBuilder<'a> {
    pub fn new(graph: &'a mut ModuleDepGraph) -> Self {
        Self {
            graph,
            current_scope: ScopeType::Module,
            scope_stack: vec![ScopeContext {
                scope_type: ScopeType::Module,
                deferred_reads: FxHashSet::default(),
            }],
            import_usage: FxHashSet::default(),
        }
    }

    /// Build the graph from an AST with enhanced tracking
    pub fn build_from_ast(&mut self, ast: &ModModule) -> Result<()> {
        debug!(
            "Building enhanced graph from AST with {} statements",
            ast.body.len()
        );

        // First pass: identify all imports and their names
        let mut import_items = Vec::new();
        for stmt in &ast.body {
            if let Some((item_id, imported_names)) = self.process_import_statement(stmt)? {
                import_items.push((item_id, imported_names));
            }
        }

        // Second pass: process all statements and track usage
        for stmt in &ast.body {
            self.process_statement(stmt)?;
        }

        // Post-process: analyze import usage patterns
        self.analyze_import_usage_patterns(&import_items)?;

        Ok(())
    }

    /// Process import statements and return their IDs and imported names
    fn process_import_statement(&mut self, stmt: &Stmt) -> Result<Option<(ItemId, Vec<String>)>> {
        match stmt {
            Stmt::Import(import_stmt) => {
                let mut all_imported = Vec::new();
                for alias in &import_stmt.names {
                    let module_name = alias.name.as_str();
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map(|n| n.as_str())
                        .unwrap_or(module_name);

                    all_imported.push(local_name.to_string());

                    // Also track root module for dotted imports
                    if !alias.asname.is_some() && module_name.contains('.') {
                        let root = module_name.split('.').next().unwrap();
                        all_imported.push(root.to_string());
                    }
                }

                // Create the import item
                let item_data = self.create_import_item_data(import_stmt);
                let item_id = self.graph.add_item(item_data);

                Ok(Some((item_id, all_imported)))
            }
            Stmt::ImportFrom(import_from) => {
                let mut all_imported = Vec::new();

                if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*" {
                    all_imported.push("*".to_string());
                } else {
                    for alias in &import_from.names {
                        let local_name = alias
                            .asname
                            .as_ref()
                            .map(|n| n.as_str())
                            .unwrap_or(alias.name.as_str());
                        all_imported.push(local_name.to_string());
                    }
                }

                // Create the import item
                let item_data = self.create_from_import_item_data(import_from);
                let item_id = self.graph.add_item(item_data);

                Ok(Some((item_id, all_imported)))
            }
            _ => Ok(None),
        }
    }

    /// Process a statement with context tracking
    fn process_statement(&mut self, stmt: &Stmt) -> Result<()> {
        match stmt {
            Stmt::FunctionDef(func_def) => self.process_function_def_with_context(func_def),
            Stmt::ClassDef(class_def) => self.process_class_def_with_context(class_def),
            Stmt::Assign(assign) => self.process_assign_with_context(assign),
            Stmt::Expr(expr_stmt) => self.process_expr_with_context(&expr_stmt.value),
            // Add other statement types as needed
            _ => Ok(()),
        }
    }

    /// Process function definition with enhanced context tracking
    fn process_function_def_with_context(&mut self, func_def: &ast::StmtFunctionDef) -> Result<()> {
        let func_name = func_def.name.to_string();

        // Variables read at function definition level (decorators, annotations)
        let mut module_level_reads = FxHashSet::default();

        // Process decorators (these are evaluated at module level)
        for decorator in &func_def.decorator_list {
            self.collect_vars_in_expr(&decorator.expression, &mut module_level_reads);
        }

        // Process annotations (these are also evaluated at module level)
        self.collect_annotation_vars(&func_def.parameters, &mut module_level_reads);
        if let Some(returns) = &func_def.returns {
            self.collect_vars_in_expr(returns, &mut module_level_reads);
        }

        // Create function item with module-level reads
        let mut item_data = ItemData {
            item_type: ItemType::FunctionDef {
                name: func_name.clone(),
            },
            var_decls: [func_name.clone()].into_iter().collect(),
            read_vars: module_level_reads,
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        };

        // Push function scope
        self.scope_stack.push(ScopeContext {
            scope_type: ScopeType::Function(func_name.clone()),
            deferred_reads: FxHashSet::default(),
        });

        // Process function body
        for stmt in &func_def.body {
            self.process_statement(stmt)?;
        }

        // Pop scope and collect deferred reads
        if let Some(scope) = self.scope_stack.pop() {
            item_data.eventual_read_vars = scope.deferred_reads;
        }

        // Add the function item
        self.graph.add_item(item_data);

        Ok(())
    }

    /// Process class definition with enhanced context tracking
    fn process_class_def_with_context(&mut self, class_def: &ast::StmtClassDef) -> Result<()> {
        let class_name = class_def.name.to_string();

        // Variables read at class definition level
        let mut module_level_reads = FxHashSet::default();

        // Process decorators and base classes
        for decorator in &class_def.decorator_list {
            self.collect_vars_in_expr(&decorator.expression, &mut module_level_reads);
        }

        if let Some(arguments) = &class_def.arguments {
            for arg in &arguments.args {
                self.collect_vars_in_expr(arg, &mut module_level_reads);
            }
        }

        // Create class item
        let item_data = ItemData {
            item_type: ItemType::ClassDef {
                name: class_name.clone(),
            },
            var_decls: [class_name.clone()].into_iter().collect(),
            read_vars: module_level_reads,
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        };

        let class_item_id = self.graph.add_item(item_data);

        // Push class scope
        self.scope_stack.push(ScopeContext {
            scope_type: ScopeType::Class(class_name),
            deferred_reads: FxHashSet::default(),
        });

        // Process class body
        for stmt in &class_def.body {
            self.process_statement(stmt)?;
        }

        // Pop scope
        self.scope_stack.pop();

        Ok(())
    }

    /// Process assignment with context tracking
    fn process_assign_with_context(&mut self, assign: &ast::StmtAssign) -> Result<()> {
        let mut read_vars = FxHashSet::default();
        self.collect_vars_in_expr(&assign.value, &mut read_vars);

        // Check current scope
        let in_function = self
            .scope_stack
            .iter()
            .any(|s| matches!(s.scope_type, ScopeType::Function(_)));

        if in_function {
            // We're inside a function - add reads to the function's deferred reads
            if let Some(current_scope) = self.scope_stack.last_mut() {
                current_scope.deferred_reads.extend(read_vars.clone());
            }
        }

        // Extract targets
        let mut targets = Vec::new();
        for target in &assign.targets {
            if let Some(names) = self.extract_assignment_targets(target) {
                targets.extend(names);
            }
        }

        let item_data = ItemData {
            item_type: ItemType::Assignment { targets },
            var_decls: FxHashSet::default(),
            read_vars: if in_function {
                FxHashSet::default()
            } else {
                read_vars
            },
            eventual_read_vars: if in_function {
                read_vars
            } else {
                FxHashSet::default()
            },
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        };

        self.graph.add_item(item_data);
        Ok(())
    }

    /// Process expression with context tracking
    fn process_expr_with_context(&mut self, expr: &Expr) -> Result<()> {
        let mut read_vars = FxHashSet::default();
        self.collect_vars_in_expr(expr, &mut read_vars);

        // Check current scope
        let in_function = self
            .scope_stack
            .iter()
            .any(|s| matches!(s.scope_type, ScopeType::Function(_)));

        if in_function {
            // Add to function's deferred reads
            if let Some(current_scope) = self.scope_stack.last_mut() {
                current_scope.deferred_reads.extend(read_vars.clone());
            }
        }

        let item_data = ItemData {
            item_type: ItemType::Expression,
            var_decls: FxHashSet::default(),
            read_vars: if in_function {
                FxHashSet::default()
            } else {
                read_vars
            },
            eventual_read_vars: if in_function {
                read_vars
            } else {
                FxHashSet::default()
            },
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: true,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        };

        self.graph.add_item(item_data);
        Ok(())
    }

    /// Analyze import usage patterns after building the graph
    fn analyze_import_usage_patterns(
        &mut self,
        import_items: &[(ItemId, Vec<String>)],
    ) -> Result<()> {
        for (import_id, imported_names) in import_items {
            for imported_name in imported_names {
                let mut module_level_usage = Vec::new();
                let mut function_level_usage = Vec::new();

                // Check all items to see where this import is used
                for (item_id, item_data) in &self.graph.items {
                    if *item_id == *import_id {
                        continue; // Skip the import itself
                    }

                    // Check module-level usage
                    if item_data.read_vars.contains(imported_name) {
                        module_level_usage.push(*item_id);
                    }

                    // Check function-level usage
                    if item_data.eventual_read_vars.contains(imported_name) {
                        function_level_usage.push(*item_id);
                    }
                }

                debug!(
                    "Import '{}' usage analysis: {} module-level, {} function-level",
                    imported_name,
                    module_level_usage.len(),
                    function_level_usage.len()
                );
            }
        }

        Ok(())
    }

    // Helper methods (simplified versions from the original graph builder)

    fn create_import_item_data(&self, import_stmt: &ast::StmtImport) -> ItemData {
        // Simplified - would need full implementation
        ItemData {
            item_type: ItemType::Import {
                module: import_stmt.names[0].name.to_string(),
                alias: import_stmt.names[0].asname.as_ref().map(|n| n.to_string()),
            },
            var_decls: FxHashSet::default(),
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        }
    }

    fn create_from_import_item_data(&self, import_from: &ast::StmtImportFrom) -> ItemData {
        // Simplified - would need full implementation
        ItemData {
            item_type: ItemType::FromImport {
                module: import_from
                    .module
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or_default(),
                names: vec![],
                level: import_from.level,
                is_star: false,
            },
            var_decls: FxHashSet::default(),
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        }
    }

    fn collect_vars_in_expr(&self, expr: &Expr, vars: &mut FxHashSet<String>) {
        // Simplified - would use the full implementation from graph_builder.rs
        match expr {
            Expr::Name(name) => {
                vars.insert(name.id.to_string());
            }
            // ... other cases
            _ => {}
        }
    }

    fn collect_annotation_vars(&self, params: &ast::Parameters, vars: &mut FxHashSet<String>) {
        // Collect from parameter annotations
        for param in &params.args {
            if let Some(annotation) = &param.parameter.annotation {
                self.collect_vars_in_expr(annotation, vars);
            }
        }
        // ... handle other parameter types
    }

    fn extract_assignment_targets(&self, expr: &Expr) -> Option<Vec<String>> {
        // Simplified - would use the full implementation
        match expr {
            Expr::Name(name) => Some(vec![name.id.to_string()]),
            _ => None,
        }
    }
}

/// Enhanced module dependency graph with better import tracking
pub trait EnhancedModuleDepGraph {
    /// Check if imports are only used inside functions with proper context tracking
    fn are_imports_used_only_in_functions_enhanced(&self) -> bool;
}

impl EnhancedModuleDepGraph for ModuleDepGraph {
    fn are_imports_used_only_in_functions_enhanced(&self) -> bool {
        // Get all imported names
        let mut imported_names = FxHashSet::default();

        for item in self.items.values() {
            match &item.item_type {
                ItemType::Import { alias, module } => {
                    imported_names.insert(alias.as_ref().unwrap_or(module).clone());
                }
                ItemType::FromImport { names, .. } => {
                    for (name, alias) in names {
                        imported_names.insert(alias.as_ref().unwrap_or(name).clone());
                    }
                }
                _ => {}
            }
        }

        debug!("Checking usage for imported names: {:?}", imported_names);

        // Check each imported name
        for imported_name in imported_names {
            // Check all items for module-level usage
            for item in self.items.values() {
                // If the import is used in read_vars (not eventual_read_vars),
                // it's a module-level usage
                if item.read_vars.contains(&imported_name) {
                    debug!(
                        "Import '{}' used at module level in {:?}",
                        imported_name, item.item_type
                    );
                    return false;
                }
            }
        }

        true
    }
}
