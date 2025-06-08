use anyhow::Result;
use cow_utils::CowUtils;
use indexmap::{IndexMap, IndexSet};
use ruff_python_ast::{self as ast, Alias, Expr, ExprContext, Identifier, Stmt};
use ruff_python_stdlib::{builtins, keyword};
use ruff_text_size::TextRange;

/// Suffix used to identify relative import modules
const REL_IMPORT_SUFFIX: &str = "_imported";

/// Scope type for tracking different kinds of scopes in Python
#[derive(Debug, Clone, PartialEq)]
pub enum ScopeType {
    Module,
    Function,
    Class,
    Comprehension,
}

/// Comprehensive symbol information
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub scope_type: ScopeType,
    pub is_parameter: bool,
    pub is_global: bool,
    pub is_nonlocal: bool,
    pub is_imported: bool,
    pub definitions: Vec<String>, // File locations where defined
    pub usages: Vec<String>,      // File locations where used
}

/// Information about an import alias that needs to be resolved
#[derive(Debug, Clone)]
pub struct ImportAlias {
    /// The original name being imported (e.g., "process_data")
    pub original_name: String,
    /// The alias name (e.g., "process_a")
    pub alias_name: String,
    /// The module the import comes from (e.g., "module_a")
    pub module_name: String,
    /// Whether this is a "from" import or a direct import
    pub is_from_import: bool,
    /// Whether this was an explicit alias in the original code (e.g., `as alias_name`)
    pub has_explicit_alias: bool,
    /// Whether the imported name is actually a module (for module imports)
    pub is_module_import: bool,
}

/// Information about name conflicts that need to be resolved
#[derive(Debug, Clone)]
pub struct NameConflict {
    /// The conflicting name
    pub name: String,
    /// Modules that define this name
    pub modules: Vec<String>,
    /// Renamed versions for each module
    pub renamed_versions: IndexMap<String, String>,
}

/// AST rewriter for handling import aliases and name conflicts
pub struct AstRewriter {
    /// Map of import aliases that need to be resolved in the entry module
    import_aliases: IndexMap<String, ImportAlias>,
    /// Map of name conflicts and their resolutions
    name_conflicts: IndexMap<String, NameConflict>,
    /// Map of renamed identifiers per module
    module_renames: IndexMap<String, IndexMap<String, String>>,
    /// Set of all reserved names (builtins, keywords, already used names)
    reserved_names: IndexSet<String>,
    /// Symbol table for comprehensive scope analysis
    symbols: IndexMap<String, Symbol>,
    /// Set of modules that are from __init__.py files
    init_modules: IndexSet<String>,
    /// Python version for builtin checks
    python_version: u8,
    /// Import strategies for each module (to know which are ModuleImport)
    import_strategies: IndexMap<String, crate::emit::ImportStrategy>, // module_name -> strategy_type
}

impl AstRewriter {
    pub fn new(python_version: u8) -> Self {
        // Initialize reserved names with Python builtins and keywords using ruff_python_stdlib
        let mut reserved_names = IndexSet::new();

        // Add all Python built-ins for the specified version
        for builtin in builtins::python_builtins(python_version, false) {
            reserved_names.insert(builtin.to_owned());
        }

        // Note: Python keywords are checked dynamically using ruff_python_stdlib::keyword::is_keyword
        // rather than pre-populating the set, for better maintainability

        Self {
            import_aliases: IndexMap::new(),
            name_conflicts: IndexMap::new(),
            module_renames: IndexMap::new(),
            reserved_names,
            symbols: IndexMap::new(),
            init_modules: IndexSet::new(),
            python_version,
            import_strategies: IndexMap::new(),
        }
    }

    /// Public getter for import_aliases (for testing)
    pub fn import_aliases(&self) -> &IndexMap<String, ImportAlias> {
        &self.import_aliases
    }

    /// Set import strategies for modules
    pub fn set_import_strategies(
        &mut self,
        strategies: &IndexMap<String, crate::emit::ImportStrategy>,
    ) {
        for (module, strategy) in strategies {
            self.import_strategies
                .insert(module.clone(), strategy.clone());
        }
    }

    /// Get module renames for a specific module
    pub fn get_module_renames(&self, module_name: &str) -> Option<&IndexMap<String, String>> {
        self.module_renames.get(module_name)
    }

    /// Set the modules that are from __init__.py files
    pub fn set_init_modules(&mut self, init_modules: &IndexSet<String>) {
        self.init_modules = init_modules.clone();
    }

    /// Collect import aliases from the entry module before they are removed
    pub fn collect_import_aliases(&mut self, entry_ast: &ast::ModModule, _entry_module_name: &str) {
        for stmt in &entry_ast.body {
            match stmt {
                Stmt::ImportFrom(import_from) => {
                    self.process_import_from_statement(import_from);
                }
                Stmt::Import(import) => {
                    self.process_import_statement(import);
                }
                _ => {}
            }
        }
    }

    /// Update module import flags based on resolver information
    pub fn update_module_import_flags<F>(&mut self, is_module_checker: F)
    where
        F: Fn(&str) -> bool,
    {
        for import_alias in self.import_aliases.values_mut() {
            if import_alias.is_from_import {
                let full_module_name = format!(
                    "{}.{}",
                    import_alias.module_name, import_alias.original_name
                );
                import_alias.is_module_import = is_module_checker(&full_module_name);
            }
        }
    }

    /// Process ImportFrom statement to extract aliases
    fn process_import_from_statement(&mut self, import_from: &ast::StmtImportFrom) {
        let Some(module) = &import_from.module else {
            return;
        };

        for alias in &import_from.names {
            let import_alias = if let Some(asname) = &alias.asname {
                // Import with explicit alias: from module import name as alias
                ImportAlias {
                    #[allow(clippy::disallowed_methods)]
                    original_name: alias.name.to_string(),
                    #[allow(clippy::disallowed_methods)]
                    alias_name: asname.to_string(),
                    #[allow(clippy::disallowed_methods)]
                    module_name: module.to_string(),
                    is_from_import: true,
                    has_explicit_alias: true,
                    is_module_import: false, // Will be set later based on resolver
                }
            } else {
                // Import without alias: from module import name
                ImportAlias {
                    #[allow(clippy::disallowed_methods)]
                    original_name: alias.name.to_string(),
                    #[allow(clippy::disallowed_methods)]
                    alias_name: alias.name.to_string(), // Same as original name
                    #[allow(clippy::disallowed_methods)]
                    module_name: module.to_string(),
                    is_from_import: true,
                    has_explicit_alias: false,
                    is_module_import: false, // Will be set later based on resolver
                }
            };

            let key = if alias.asname.is_some() {
                import_alias.alias_name.clone()
            } else {
                #[allow(clippy::disallowed_methods)]
                alias.name.to_string()
            };
            self.import_aliases.insert(key, import_alias);
        }
    }

    /// Process Import statement to extract aliases
    fn process_import_statement(&mut self, import: &ast::StmtImport) {
        for alias in &import.names {
            if let Some(asname) = &alias.asname {
                let import_alias = ImportAlias {
                    #[allow(clippy::disallowed_methods)]
                    original_name: alias.name.to_string(),
                    #[allow(clippy::disallowed_methods)]
                    alias_name: asname.to_string(),
                    #[allow(clippy::disallowed_methods)]
                    module_name: alias.name.to_string(),
                    is_from_import: false,
                    has_explicit_alias: true,
                    is_module_import: true, // Regular imports are always module imports
                };
                #[allow(clippy::disallowed_methods)]
                self.import_aliases.insert(asname.to_string(), import_alias);
            }
        }
    }

    /// Collect symbols from all modules for comprehensive analysis
    pub fn collect_symbols(&mut self, modules: &[(String, &ast::ModModule)]) {
        for (module_name, module_ast) in modules {
            self.collect_module_symbols(module_name, module_ast);
        }
    }

    /// Collect symbols from a single module
    fn collect_module_symbols(&mut self, module_name: &str, module_ast: &ast::ModModule) {
        for stmt in &module_ast.body {
            self.collect_symbols_from_stmt(module_name, stmt, &ScopeType::Module);
        }
    }

    /// Collect symbols from a statement
    fn collect_symbols_from_stmt(
        &mut self,
        module_name: &str,
        stmt: &Stmt,
        scope_type: &ScopeType,
    ) {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                let symbol_key = format!("{}::{}", module_name, func_def.name);
                let symbol = Symbol {
                    #[allow(clippy::disallowed_methods)]
                    name: func_def.name.to_string(),
                    scope_type: scope_type.clone(),
                    is_parameter: false,
                    is_global: matches!(scope_type, ScopeType::Module),
                    is_nonlocal: false,
                    is_imported: false,
                    definitions: vec![module_name.to_owned()],
                    usages: vec![],
                };
                self.symbols.insert(symbol_key, symbol);

                // Collect symbols from function body
                for body_stmt in &func_def.body {
                    self.collect_symbols_from_stmt(module_name, body_stmt, &ScopeType::Function);
                }
            }
            Stmt::ClassDef(class_def) => {
                let symbol_key = format!("{}::{}", module_name, class_def.name);
                let symbol = Symbol {
                    #[allow(clippy::disallowed_methods)]
                    name: class_def.name.to_string(),
                    scope_type: scope_type.clone(),
                    is_parameter: false,
                    is_global: matches!(scope_type, ScopeType::Module),
                    is_nonlocal: false,
                    is_imported: false,
                    definitions: vec![module_name.to_owned()],
                    usages: vec![],
                };
                self.symbols.insert(symbol_key, symbol);

                // Collect symbols from class body
                for body_stmt in &class_def.body {
                    self.collect_symbols_from_stmt(module_name, body_stmt, &ScopeType::Class);
                }
            }
            Stmt::Assign(assign) => {
                for target in &assign.targets {
                    self.collect_symbols_from_expr(module_name, target, scope_type, true);
                }
                self.collect_symbols_from_expr(module_name, &assign.value, scope_type, false);
            }
            Stmt::Expr(expr_stmt) => {
                self.collect_symbols_from_expr(module_name, &expr_stmt.value, scope_type, false);
            }
            _ => {}
        }
    }

    /// Collect symbols from an expression
    #[allow(clippy::too_many_arguments)]
    fn collect_symbols_from_expr(
        &mut self,
        module_name: &str,
        expr: &Expr,
        scope_type: &ScopeType,
        is_assignment: bool,
    ) {
        match expr {
            Expr::Name(name) => {
                // Skip built-ins using ruff_python_stdlib
                if builtins::is_python_builtin(&name.id, self.python_version, false) {
                    return;
                }

                let symbol_key = format!("{}::{}", module_name, name.id);
                if is_assignment && matches!(scope_type, ScopeType::Module) {
                    // This is a module-level assignment
                    let symbol = Symbol {
                        #[allow(clippy::disallowed_methods)]
                        name: name.id.to_string(),
                        scope_type: scope_type.clone(),
                        is_parameter: false,
                        is_global: true,
                        is_nonlocal: false,
                        is_imported: false,
                        definitions: vec![module_name.to_owned()],
                        usages: vec![],
                    };
                    self.symbols.insert(symbol_key, symbol);
                }
            }
            Expr::Call(call) => {
                self.collect_symbols_from_expr(module_name, &call.func, scope_type, false);
                for arg in &call.arguments.args {
                    self.collect_symbols_from_expr(module_name, arg, scope_type, false);
                }
            }
            Expr::Attribute(attr) => {
                self.collect_symbols_from_expr(module_name, &attr.value, scope_type, false);
            }
            _ => {}
        }
    }

    /// Analyze modules to detect name conflicts
    pub fn analyze_name_conflicts(&mut self, modules: &[(String, &ast::ModModule)]) {
        // Store module dependency information
        // For now, disable the relative import exclusion logic entirely
        // The original problem was with specific relative import patterns in packages
        // We need a more targeted approach that only applies to actual relative imports

        // Build symbol conflicts first
        self.collect_symbols(modules);

        let name_to_modules = self.collect_module_level_identifiers(modules);
        self.generate_conflict_resolutions(name_to_modules);
    }

    /// Collect all module-level identifiers that could potentially conflict
    fn collect_module_level_identifiers(
        &self,
        modules: &[(String, &ast::ModModule)],
    ) -> IndexMap<String, Vec<String>> {
        let mut name_to_modules: IndexMap<String, Vec<String>> = IndexMap::new();

        for (module_name, _) in modules {
            self.collect_symbols_for_module(module_name, &mut name_to_modules);
        }

        name_to_modules
    }

    /// Collect symbols for a specific module
    fn collect_symbols_for_module(
        &self,
        module_name: &str,
        name_to_modules: &mut IndexMap<String, Vec<String>>,
    ) {
        for (symbol_key, symbol) in &self.symbols {
            if self.is_conflictable_symbol(symbol_key, symbol, module_name) {
                name_to_modules
                    .entry(symbol.name.clone())
                    .or_default()
                    .push(module_name.to_owned());
            }
        }
    }

    /// Check if a symbol can potentially conflict
    fn is_conflictable_symbol(&self, symbol_key: &str, symbol: &Symbol, module_name: &str) -> bool {
        // Special case: __all__ should not be treated as conflictable
        // It's metadata that guides exports, not a regular variable
        if symbol.name == "__all__" {
            return false;
        }

        symbol_key.starts_with(&format!("{}::", module_name))
            && symbol.is_global
            && !symbol.is_imported
    }

    /// Generate conflict resolutions for conflicting names
    fn generate_conflict_resolutions(&mut self, name_to_modules: IndexMap<String, Vec<String>>) {
        for (name, modules) in name_to_modules {
            if modules.len() <= 1 {
                continue;
            }
            // Special handling for __init__.py package interfaces
            let mut handled = false;
            if let Some(package_module) = self.find_package_interface_module(&modules) {
                handled = self.handle_submodule_conflicts(&name, &modules, package_module.as_str());
            }
            if handled {
                continue;
            }
            // Default conflict resolution for all other cases
            self.resolve_name_conflict(&name, &modules);
        }
    }

    /// Find if one of the modules is a package interface (__init__.py) for the others
    fn find_package_interface_module(&self, modules: &[String]) -> Option<String> {
        for module in modules {
            // Use the actual init_modules set instead of heuristic check
            if !self.init_modules.contains(module) {
                continue;
            }
            // Check if other modules are submodules of this package
            let package_prefix = format!("{}.", module);
            if modules
                .iter()
                .any(|m| m.as_str() != module && m.starts_with(&package_prefix))
            {
                return Some(module.clone());
            }
        }
        None
    }

    /// Resolve conflicts for submodules only, leaving package interface unchanged
    fn resolve_submodule_conflicts(&mut self, name: &str, submodules: &[String]) {
        let mut renamed_versions = IndexMap::new();

        for module in submodules {
            let renamed = self.generate_unique_name(name, module);
            renamed_versions.insert(module.clone(), renamed.clone());

            // Track renames for this module
            self.module_renames
                .entry(module.clone())
                .or_default()
                .insert(name.to_owned(), renamed);
        }

        let conflict = NameConflict {
            name: name.to_owned(),
            modules: submodules.to_vec(),
            renamed_versions,
        };
        self.name_conflicts.insert(name.to_owned(), conflict);
    }

    /// Resolve a specific name conflict
    fn resolve_name_conflict(&mut self, name: &str, modules: &[String]) {
        let mut renamed_versions = IndexMap::new();

        for module in modules {
            let renamed = self.generate_unique_name(name, module);
            renamed_versions.insert(module.clone(), renamed.clone());

            // Track renames for this module
            self.module_renames
                .entry(module.clone())
                .or_default()
                .insert(name.to_string(), renamed);
        }

        let conflict = NameConflict {
            name: name.to_string(),
            modules: modules.to_vec(),
            renamed_versions,
        };
        self.name_conflicts.insert(name.to_owned(), conflict);
    }

    /// Generate a unique name for a conflicting identifier
    fn generate_unique_name(&mut self, original_name: &str, module_name: &str) -> String {
        // Clean up module name for use as prefix
        let module_prefix = module_name
            .cow_replace(".", "_")
            .cow_replace("-", "_")
            .cow_replace("/", "_")
            .into_owned();

        let mut counter = 0;

        loop {
            let candidate = if counter == 0 {
                format!("__{}_{}", module_prefix, original_name)
            } else {
                format!("__{}_{}_{}", module_prefix, original_name, counter)
            };

            // Check if the name is available
            if !self.is_reserved_name(&candidate) && !self.is_name_used_in_any_module(&candidate) {
                // Reserve the name
                self.reserved_names.insert(candidate.clone());
                return candidate;
            }

            counter += 1;
        }
    }

    /// Check if a name is reserved (builtin, keyword, or manually reserved)
    fn is_reserved_name(&self, name: &str) -> bool {
        // Check if it's a Python keyword using ruff_python_stdlib
        if keyword::is_keyword(name) {
            return true;
        }

        // Check if it's in our manually maintained reserved names (builtins + user-reserved)
        self.reserved_names.contains(name)
    }

    /// Check if a name is used in any module
    fn is_name_used_in_any_module(&self, name: &str) -> bool {
        // Check if the name exists in any module's rename map
        for renames in self.module_renames.values() {
            if renames.contains_key(name) || renames.values().any(|v| v == name) {
                return true;
            }
        }

        // Check if the name exists in symbols
        self.symbols.contains_key(name)
    }

    /// Rewrite a module's AST to resolve name conflicts
    pub fn rewrite_module_ast(
        &self,
        module_name: &str,
        module_ast: &mut ast::ModModule,
    ) -> Result<()> {
        log::debug!(
            "rewrite_module_ast called for module '{}', has renames: {}",
            module_name,
            self.module_renames.contains_key(module_name)
        );
        if let Some(renames) = self.module_renames.get(module_name) {
            log::debug!(
                "Applying {} renames to module '{}': {:?}",
                renames.len(),
                module_name,
                renames
            );
            self.apply_renames_to_ast(&mut module_ast.body, renames)?;
        } else {
            log::debug!("No renames found for module '{}'", module_name);
        }
        Ok(())
    }

    /// Apply renames to an AST node recursively
    fn apply_renames_to_ast(
        &self,
        statements: &mut Vec<Stmt>,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        for stmt in statements {
            self.apply_renames_to_stmt(stmt, renames)?;
        }
        Ok(())
    }

    /// Apply renames to a list of generators (used in comprehensions)
    fn apply_renames_to_generators(
        &self,
        generators: &mut [ast::Comprehension],
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        for generator in generators {
            self.apply_renames_to_expr(&mut generator.target, renames)?;
            self.apply_renames_to_expr(&mut generator.iter, renames)?;
            for if_ in &mut generator.ifs {
                self.apply_renames_to_expr(if_, renames)?;
            }
        }
        Ok(())
    }

    /// Apply renames to a single statement
    fn apply_renames_to_stmt(
        &self,
        stmt: &mut Stmt,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        match stmt {
            Stmt::FunctionDef(func_def) => self.apply_renames_to_function_def(func_def, renames),
            Stmt::ClassDef(class_def) => self.apply_renames_to_class_def(class_def, renames),
            Stmt::Assign(assign) => self.apply_renames_to_assign(assign, renames),
            Stmt::Expr(expr_stmt) => self.apply_renames_to_expr(&mut expr_stmt.value, renames),
            Stmt::Return(return_stmt) => self.apply_renames_to_return(return_stmt, renames),
            Stmt::If(if_stmt) => self.apply_renames_to_if(if_stmt, renames),
            Stmt::While(while_stmt) => self.apply_renames_to_while(while_stmt, renames),
            Stmt::For(for_stmt) => self.apply_renames_to_for(for_stmt, renames),
            Stmt::With(with_stmt) => self.apply_renames_to_with(with_stmt, renames),
            Stmt::Try(try_stmt) => self.apply_renames_to_try(try_stmt, renames),
            Stmt::AugAssign(aug_assign) => self.apply_renames_to_aug_assign(aug_assign, renames),
            Stmt::AnnAssign(ann_assign) => self.apply_renames_to_ann_assign(ann_assign, renames),
            _ => Ok(()),
        }
    }

    /// Apply renames to function definition statement
    fn apply_renames_to_function_def(
        &self,
        func_def: &mut ast::StmtFunctionDef,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        if let Some(new_name) = renames.get(func_def.name.as_str()) {
            func_def.name = Identifier::new(new_name.clone(), TextRange::default());
        }
        if let Some(returns) = &mut func_def.returns {
            self.apply_renames_to_expr(returns, renames)?;
        }
        self.apply_renames_to_ast(&mut func_def.body, renames)
    }

    /// Apply renames to class definition statement
    fn apply_renames_to_class_def(
        &self,
        class_def: &mut ast::StmtClassDef,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        if let Some(new_name) = renames.get(class_def.name.as_str()) {
            class_def.name = Identifier::new(new_name.clone(), TextRange::default());
        }
        self.apply_renames_to_ast(&mut class_def.body, renames)
    }

    /// Apply renames to assignment statement
    fn apply_renames_to_assign(
        &self,
        assign: &mut ast::StmtAssign,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        // Only rename assignment targets if they are not simple variable names
        // Simple variable names in assignment targets represent new bindings (like in relative imports)
        // and should not be renamed. Complex expressions (attributes, subscripts) may reference
        // existing variables and should be renamed.
        // Exception: Simple variable names that are part of conflict-based renames should be renamed.
        for target in &mut assign.targets {
            let is_simple = self.is_simple_variable_binding(target);
            let should_rename_simple = self.should_rename_simple_binding(target, renames);

            if let Expr::Name(name_expr) = target {
                log::debug!(
                    "apply_renames_to_assign: target='{}', is_simple={}, should_rename_simple={}",
                    name_expr.id,
                    is_simple,
                    should_rename_simple
                );
            }

            if !is_simple || should_rename_simple {
                self.apply_renames_to_expr(target, renames)?;
            }
        }
        // Always rename the value (right-hand side) as it references existing variables
        self.apply_renames_to_expr(&mut assign.value, renames)
    }

    /// Check if an expression represents a simple variable binding (should not be renamed)
    /// Returns true for simple names like 'x', false for complex expressions like 'obj.attr' or 'arr[i]'
    fn is_simple_variable_binding(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Name(_))
    }

    /// Check if a simple variable binding should be renamed due to conflict-based rename
    fn should_rename_simple_binding(
        &self,
        target: &Expr,
        renames: &IndexMap<String, String>,
    ) -> bool {
        if let Expr::Name(name_expr) = target {
            if let Some(renamed_to) = renames.get(&name_expr.id.to_string()) {
                let is_conflict_based =
                    self.is_conflict_based_rename_mapping(&name_expr.id, renamed_to);
                log::debug!(
                    "should_rename_simple_binding: '{}' -> '{}', is_conflict_based: {}",
                    name_expr.id,
                    renamed_to,
                    is_conflict_based
                );
                return is_conflict_based;
            }
        }
        false
    }

    /// Check if a rename mapping represents a conflict-based rename
    /// Conflict-based renames follow the pattern: original_name -> __module_original_name
    fn is_conflict_based_rename_mapping(&self, original_name: &str, renamed_name: &str) -> bool {
        // Check if the renamed name follows the conflict resolution pattern
        let Some(suffix_start) = renamed_name.strip_prefix("__") else {
            return false;
        };

        // Check if the suffix ends with the original name
        // This handles variable names that contain underscores correctly
        if !suffix_start.ends_with(original_name) {
            log::debug!(
                "is_conflict_based_rename_mapping: '{}' -> '{}', suffix_start='{}', ends_with_original=false, is_match=false",
                original_name,
                renamed_name,
                suffix_start
            );
            return false;
        }

        // Verify there's a separator (underscore) before the original name
        let expected_prefix_len = suffix_start.len() - original_name.len();
        let has_separator = expected_prefix_len > 0
            && suffix_start.chars().nth(expected_prefix_len - 1) == Some('_');

        if has_separator {
            log::debug!(
                "is_conflict_based_rename_mapping: '{}' -> '{}', suffix_start='{}', ends_with_original=true, has_separator=true, is_match=true",
                original_name,
                renamed_name,
                suffix_start
            );
            true
        } else {
            log::debug!(
                "is_conflict_based_rename_mapping: '{}' -> '{}', suffix_start='{}', ends_with_original=true, has_separator=false, is_match=false",
                original_name,
                renamed_name,
                suffix_start
            );
            false
        }
    }

    /// Apply renames to return statement
    fn apply_renames_to_return(
        &self,
        return_stmt: &mut ast::StmtReturn,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        if let Some(value) = &mut return_stmt.value {
            self.apply_renames_to_expr(value, renames)?;
        }
        Ok(())
    }

    /// Apply renames to if statement
    fn apply_renames_to_if(
        &self,
        if_stmt: &mut ast::StmtIf,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut if_stmt.test, renames)?;
        self.apply_renames_to_ast(&mut if_stmt.body, renames)?;

        for clause in &mut if_stmt.elif_else_clauses {
            if let Some(condition) = &mut clause.test {
                self.apply_renames_to_expr(condition, renames)?;
            }
            self.apply_renames_to_ast(&mut clause.body, renames)?;
        }
        Ok(())
    }

    /// Apply renames to while statement
    fn apply_renames_to_while(
        &self,
        while_stmt: &mut ast::StmtWhile,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut while_stmt.test, renames)?;
        self.apply_renames_to_ast(&mut while_stmt.body, renames)?;
        self.apply_renames_to_ast(&mut while_stmt.orelse, renames)
    }

    /// Apply renames to for statement
    fn apply_renames_to_for(
        &self,
        for_stmt: &mut ast::StmtFor,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut for_stmt.target, renames)?;
        self.apply_renames_to_expr(&mut for_stmt.iter, renames)?;
        self.apply_renames_to_ast(&mut for_stmt.body, renames)?;
        self.apply_renames_to_ast(&mut for_stmt.orelse, renames)
    }

    /// Apply renames to with statement
    fn apply_renames_to_with(
        &self,
        with_stmt: &mut ast::StmtWith,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        for item in &mut with_stmt.items {
            self.apply_renames_to_expr(&mut item.context_expr, renames)?;
            if let Some(optional_vars) = &mut item.optional_vars {
                self.apply_renames_to_expr(optional_vars, renames)?;
            }
        }
        self.apply_renames_to_ast(&mut with_stmt.body, renames)
    }

    /// Apply renames to try statement
    fn apply_renames_to_try(
        &self,
        try_stmt: &mut ast::StmtTry,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_ast(&mut try_stmt.body, renames)?;

        for handler in &mut try_stmt.handlers {
            self.apply_renames_to_exception_handler(handler, renames)?;
        }

        self.apply_renames_to_ast(&mut try_stmt.orelse, renames)?;
        self.apply_renames_to_ast(&mut try_stmt.finalbody, renames)
    }

    /// Apply renames to an exception handler
    fn apply_renames_to_exception_handler(
        &self,
        handler: &mut ast::ExceptHandler,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        let ast::ExceptHandler::ExceptHandler(except_data) = handler;

        if let Some(type_) = &mut except_data.type_ {
            self.apply_renames_to_expr(type_, renames)?;
        }

        self.apply_renames_to_exception_name(&mut except_data.name, renames);
        self.apply_renames_to_ast(&mut except_data.body, renames)
    }

    /// Apply renames to exception handler name
    fn apply_renames_to_exception_name(
        &self,
        name: &mut Option<Identifier>,
        renames: &IndexMap<String, String>,
    ) {
        if let Some(name_ident) = name {
            if let Some(new_name) = renames.get(name_ident.as_str()) {
                *name_ident = Identifier::new(new_name.clone(), TextRange::default());
            }
        }
    }

    /// Apply renames to augmented assignment statement
    fn apply_renames_to_aug_assign(
        &self,
        aug_assign: &mut ast::StmtAugAssign,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut aug_assign.target, renames)?;
        self.apply_renames_to_expr(&mut aug_assign.value, renames)
    }

    /// Apply renames to annotated assignment statement
    fn apply_renames_to_ann_assign(
        &self,
        ann_assign: &mut ast::StmtAnnAssign,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut ann_assign.target, renames)?;
        self.apply_renames_to_expr(&mut ann_assign.annotation, renames)?;
        if let Some(value) = &mut ann_assign.value {
            self.apply_renames_to_expr(value, renames)?;
        }
        Ok(())
    }

    /// Apply renames to an expression
    fn apply_renames_to_expr(
        &self,
        expr: &mut Expr,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        match expr {
            Expr::Name(name) => self.apply_renames_to_name(name, renames),
            Expr::Call(call) => self.apply_renames_to_call(call, renames),
            Expr::Attribute(attr) => self.apply_renames_to_expr(&mut attr.value, renames),
            Expr::BinOp(binop) => self.apply_renames_to_binop(binop, renames),
            Expr::UnaryOp(unary) => self.apply_renames_to_expr(&mut unary.operand, renames),
            Expr::Compare(compare) => self.apply_renames_to_compare(compare, renames),
            Expr::List(list) => self.apply_renames_to_list(list, renames),
            Expr::Set(set) => self.apply_renames_to_collection(&mut set.elts, renames),
            Expr::Tuple(tuple) => self.apply_renames_to_collection(&mut tuple.elts, renames),
            Expr::BoolOp(bool_op) => self.apply_renames_to_collection(&mut bool_op.values, renames),
            Expr::If(if_exp) => {
                self.apply_renames_to_expr(&mut if_exp.test, renames)?;
                self.apply_renames_to_expr(&mut if_exp.body, renames)?;
                self.apply_renames_to_expr(&mut if_exp.orelse, renames)
            }
            Expr::ListComp(list_comp) => {
                self.apply_renames_to_expr(&mut list_comp.elt, renames)?;
                self.apply_renames_to_generators(&mut list_comp.generators, renames)
            }
            Expr::SetComp(set_comp) => {
                self.apply_renames_to_expr(&mut set_comp.elt, renames)?;
                self.apply_renames_to_generators(&mut set_comp.generators, renames)
            }
            Expr::DictComp(dict_comp) => {
                self.apply_renames_to_expr(&mut dict_comp.key, renames)?;
                self.apply_renames_to_expr(&mut dict_comp.value, renames)?;
                self.apply_renames_to_generators(&mut dict_comp.generators, renames)
            }
            Expr::Generator(gen_exp) => {
                self.apply_renames_to_expr(&mut gen_exp.elt, renames)?;
                self.apply_renames_to_generators(&mut gen_exp.generators, renames)
            }
            Expr::Subscript(subscript) => {
                self.apply_renames_to_expr(&mut subscript.value, renames)?;
                self.apply_renames_to_expr(&mut subscript.slice, renames)
            }
            Expr::Starred(starred) => self.apply_renames_to_expr(&mut starred.value, renames),
            Expr::Slice(slice) => {
                if let Some(lower) = &mut slice.lower {
                    self.apply_renames_to_expr(lower, renames)?;
                }
                if let Some(upper) = &mut slice.upper {
                    self.apply_renames_to_expr(upper, renames)?;
                }
                if let Some(step) = &mut slice.step {
                    self.apply_renames_to_expr(step, renames)?;
                }
                Ok(())
            }
            // Add more expression types as needed
            _ => Ok(()),
        }
    }

    /// Apply renames to a name expression
    fn apply_renames_to_name(
        &self,
        name: &mut ast::ExprName,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        if let Some(new_name) = renames.get(name.id.as_str()) {
            log::debug!("Renaming '{}' to '{}'", name.id.as_str(), new_name);
            name.id = new_name.clone().into();
        }
        Ok(())
    }

    /// Apply renames to a call expression
    fn apply_renames_to_call(
        &self,
        call: &mut ast::ExprCall,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut call.func, renames)?;
        for arg in &mut call.arguments.args {
            self.apply_renames_to_expr(arg, renames)?;
        }
        for keyword in &mut call.arguments.keywords {
            self.apply_renames_to_expr(&mut keyword.value, renames)?;
        }
        Ok(())
    }

    /// Apply renames to a binary operation expression
    fn apply_renames_to_binop(
        &self,
        binop: &mut ast::ExprBinOp,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut binop.left, renames)?;
        self.apply_renames_to_expr(&mut binop.right, renames)
    }

    /// Apply renames to a comparison expression
    fn apply_renames_to_compare(
        &self,
        compare: &mut ast::ExprCompare,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut compare.left, renames)?;
        for comparator in &mut compare.comparators {
            self.apply_renames_to_expr(comparator, renames)?;
        }
        Ok(())
    }

    /// Apply renames to a list expression
    fn apply_renames_to_list(
        &self,
        list: &mut ast::ExprList,
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_collection(&mut list.elts, renames)
    }

    /// Apply renames to a collection of expressions
    fn apply_renames_to_collection(
        &self,
        elts: &mut [Expr],
        renames: &IndexMap<String, String>,
    ) -> Result<()> {
        for elt in elts {
            self.apply_renames_to_expr(elt, renames)?;
        }
        Ok(())
    }

    /// Generate alias assignments for the entry module
    pub fn generate_alias_assignments(&self) -> Vec<Stmt> {
        let mut assignments = Vec::new();

        for (alias_name, import_alias) in &self.import_aliases {
            if import_alias.is_from_import {
                self.process_from_import_alias(alias_name, import_alias, &mut assignments);
            } else {
                self.process_regular_import_alias(alias_name, import_alias, &mut assignments);
            }
        }

        assignments
    }

    /// Process from import alias (e.g., from module import item as alias)
    fn process_from_import_alias(
        &self,
        alias_name: &str,
        import_alias: &ImportAlias,
        assignments: &mut Vec<Stmt>,
    ) {
        let has_conflict = self
            .name_conflicts
            .contains_key(&import_alias.original_name);

        // Check if the source module uses ModuleImport strategy
        let needs_namespace_reference =
            if let Some(strategy) = self.import_strategies.get(&import_alias.module_name) {
                matches!(strategy, crate::emit::ImportStrategy::ModuleImport)
            } else {
                false
            };

        // Generate assignment if:
        // 1. There's an explicit alias, OR
        // 2. There's a name conflict, OR
        // 3. The source module uses ModuleImport strategy (needs namespace reference)
        if import_alias.has_explicit_alias || has_conflict || needs_namespace_reference {
            let actual_name = self.resolve_actual_name_for_conflict(import_alias);
            let assignment = self.create_from_import_assignment(alias_name, &actual_name);
            assignments.push(Stmt::Assign(assignment));
        }
    }

    /// Process regular import alias (e.g., import module as alias)
    fn process_regular_import_alias(
        &self,
        alias_name: &str,
        import_alias: &ImportAlias,
        assignments: &mut Vec<Stmt>,
    ) {
        if import_alias.has_explicit_alias {
            let assignment = self.create_regular_import_assignment(alias_name, import_alias);
            assignments.push(Stmt::Assign(assignment));
        }
    }

    /// Resolve the actual name for an import considering name conflicts and import strategies
    fn resolve_actual_name_for_conflict(&self, import_alias: &ImportAlias) -> String {
        if let Some(_conflict) = self.name_conflicts.get(&import_alias.original_name) {
            // Even when there's a conflict, we need to consider the import strategy
            // The conflict resolution only affects the target variable name, not the source reference
            if import_alias.is_from_import && import_alias.is_module_import {
                // This is a module import (e.g., from greetings import greeting)
                // Use the full module path (e.g., greetings.greeting)
                format!(
                    "{}.{}",
                    import_alias.module_name, import_alias.original_name
                )
            } else if import_alias.is_from_import {
                // This is a value import from a module - use import strategy logic
                self.resolve_from_import_reference(import_alias)
            } else {
                // This is a regular import - the conflict doesn't affect the source
                import_alias.original_name.clone()
            }
        } else {
            // For from imports, check if this is a module import
            if import_alias.is_from_import && import_alias.is_module_import {
                // This is a module import (e.g., from greetings import greeting)
                // Use the full module path (e.g., greetings.greeting)
                format!(
                    "{}.{}",
                    import_alias.module_name, import_alias.original_name
                )
            } else if import_alias.is_from_import {
                // This is a value import from a module
                self.resolve_from_import_reference(import_alias)
            } else {
                // This is a regular import
                import_alias.original_name.clone()
            }
        }
    }

    /// Resolve the reference for a from import considering import strategies
    fn resolve_from_import_reference(&self, import_alias: &ImportAlias) -> String {
        // Check if the source module was bundled with a specific import strategy
        if let Some(strategy) = self.import_strategies.get(&import_alias.module_name) {
            match strategy {
                crate::emit::ImportStrategy::ModuleImport => {
                    // The module was bundled with namespace, so reference it as module.item
                    format!(
                        "{}.{}",
                        import_alias.module_name, import_alias.original_name
                    )
                }
                crate::emit::ImportStrategy::FromImport
                | crate::emit::ImportStrategy::Dependency => {
                    // Module was inlined directly, check for name conflicts
                    self.resolve_alias_name_for_inlined_module(import_alias)
                }
            }
        } else {
            // No strategy info available, use the original name as fallback
            import_alias.original_name.clone()
        }
    }

    /// Create an assignment statement for a from import
    fn create_from_import_assignment(
        &self,
        alias_name: &str,
        actual_name: &str,
    ) -> ast::StmtAssign {
        ast::StmtAssign {
            targets: vec![Expr::Name(ast::ExprName {
                id: alias_name.to_string().into(),
                ctx: ExprContext::Store,
                range: Default::default(),
            })],
            value: Box::new(Expr::Name(ast::ExprName {
                id: actual_name.to_string().into(),
                ctx: ExprContext::Load,
                range: Default::default(),
            })),
            range: Default::default(),
        }
    }

    /// Create an assignment statement for a regular import
    fn create_regular_import_assignment(
        &self,
        alias_name: &str,
        import_alias: &ImportAlias,
    ) -> ast::StmtAssign {
        ast::StmtAssign {
            targets: vec![Expr::Name(ast::ExprName {
                id: alias_name.to_string().into(),
                ctx: ExprContext::Store,
                range: Default::default(),
            })],
            value: Box::new(Expr::Name(ast::ExprName {
                id: import_alias.module_name.clone().into(),
                ctx: ExprContext::Load,
                range: Default::default(),
            })),
            range: Default::default(),
        }
    }

    /// Get debug information about conflicts and aliases
    pub fn get_debug_info(&self) -> String {
        let mut info = String::new();

        info.push_str(&format!(
            "Import Aliases: {} found\n",
            self.import_aliases.len()
        ));
        for (alias, import_info) in &self.import_aliases {
            info.push_str(&format!(
                "  {} -> {} from {}\n",
                alias, import_info.original_name, import_info.module_name
            ));
        }

        info.push_str(&format!(
            "\nName Conflicts: {} found\n",
            self.name_conflicts.len()
        ));
        for (name, conflict) in &self.name_conflicts {
            info.push_str(&format!("  {}: {}\n", name, conflict.modules.join(", ")));
            for (module, renamed) in &conflict.renamed_versions {
                info.push_str(&format!("    {} -> {}\n", module, renamed));
            }
        }

        info
    }

    /// Transform relative imports in modules to use bundled variable references
    pub fn transform_init_py_relative_imports(
        &self,
        module_name: &str,
        module_ast: &mut ast::ModModule,
        bundled_modules: &IndexMap<String, String>,
    ) -> Result<()> {
        log::debug!(
            "transform_init_py_relative_imports called for module: {}, is_init: {}",
            module_name,
            self.init_modules.contains(module_name)
        );

        // Check if this module has any relative imports before proceeding
        let has_relative_imports = module_ast
            .body
            .iter()
            .any(|stmt| matches!(stmt, Stmt::ImportFrom(import_from) if import_from.level > 0));

        if !has_relative_imports {
            log::debug!("No relative imports found in module: {}", module_name);
            return Ok(());
        }

        log::debug!("Transforming relative imports for module: {}", module_name);

        // Use the provided bundled_modules mapping instead of constructing internally
        let mut imported_modules = IndexMap::new();
        let mut statements_to_remove = Vec::new();

        // Find relative import statements and map them to bundled variables
        for (i, stmt) in module_ast.body.iter().enumerate() {
            let Stmt::ImportFrom(import_from) = stmt else {
                continue;
            };

            // Skip non-relative imports
            if import_from.level == 0 {
                continue;
            }

            if let Some(relative_module) = &import_from.module {
                // This is a relative import like "from .greeting import message" or "from ..messages import message"
                log::debug!(
                    "Found relative import: from {}{} import [...] (level={})",
                    ".".repeat(import_from.level as usize),
                    relative_module,
                    import_from.level
                );
                self.process_relative_module_import(
                    &import_from.names,
                    (
                        relative_module.as_str(),
                        module_name,
                        import_from.level,
                        bundled_modules,
                    ),
                    &mut imported_modules,
                );
            } else {
                // This is a relative import like "from . import module"
                self.process_relative_dot_import(
                    &import_from.names,
                    (module_name, bundled_modules),
                    &mut imported_modules,
                );
            }
            statements_to_remove.push(i);
        }

        // Remove relative import statements
        for &index in statements_to_remove.iter().rev() {
            module_ast.body.remove(index);
        }

        // Add assignment statements for imported variables ONLY if we found relative imports
        // Skip module imports (those ending with REL_IMPORT_SUFFIX) as they are only used for attribute access transformation
        if !imported_modules.is_empty() {
            let assignments_to_add =
                self.create_variable_assignments_for_imports(&imported_modules);

            log::debug!("Total imported_modules: {:?}", imported_modules);

            // Insert assignments at the beginning of the module
            for (i, assignment) in assignments_to_add.into_iter().enumerate() {
                module_ast.body.insert(i, assignment);
            }
        }

        // Transform attribute access expressions (e.g., messages.message -> __greetings_messages_message)
        if !imported_modules.is_empty() {
            self.transform_attribute_access_in_statements(
                &mut module_ast.body,
                &imported_modules,
                bundled_modules,
            )?;
        }

        Ok(())
    }

    /// Create variable assignments for imported modules, skipping module imports
    fn create_variable_assignments_for_imports(
        &self,
        imported_modules: &IndexMap<String, String>,
    ) -> Vec<Stmt> {
        let mut assignments_to_add = Vec::new();
        for (imported_name, bundled_name) in imported_modules {
            // Skip module imports - they are placeholders for attribute access transformation
            if bundled_name.ends_with(REL_IMPORT_SUFFIX) {
                log::debug!(
                    "Skipping assignment for module import: {} = {} (module import placeholder)",
                    imported_name,
                    bundled_name
                );
                continue;
            }

            let assignment = self.create_variable_assignment(imported_name, bundled_name);
            assignments_to_add.push(assignment);
            log::debug!(
                "Added assignment for relative import: {} = {}",
                imported_name,
                bundled_name
            );
        }
        assignments_to_add
    }

    /// Transform attribute access expressions in statements
    fn transform_attribute_access_in_statements(
        &self,
        statements: &mut [Stmt],
        imported_modules: &IndexMap<String, String>,
        bundled_modules: &IndexMap<String, String>,
    ) -> Result<()> {
        for stmt in statements {
            self.transform_attribute_access_in_stmt(stmt, imported_modules, bundled_modules)?;
        }
        Ok(())
    }

    /// Process assignment targets for transformation, reducing nesting in transform_attribute_access_in_stmt
    fn process_assignment_targets(
        &self,
        targets: &mut Vec<Expr>,
        imported_modules: &IndexMap<String, String>,
        bundled_modules: &IndexMap<String, String>,
    ) -> Result<()> {
        for target in targets {
            if self.should_transform_assignment_target(target, imported_modules) {
                self.transform_attribute_access_in_expr(target, imported_modules, bundled_modules)?;
            }
        }
        Ok(())
    }

    /// Create a variable assignment statement: name = value
    fn create_variable_assignment(&self, name: &str, value: &str) -> Stmt {
        Stmt::Assign(ast::StmtAssign {
            targets: vec![Expr::Name(ast::ExprName {
                id: name.to_string().into(),
                ctx: ExprContext::Store,
                range: Default::default(),
            })],
            value: Box::new(Expr::Name(ast::ExprName {
                id: value.to_string().into(),
                ctx: ExprContext::Load,
                range: Default::default(),
            })),
            range: Default::default(),
        })
    }

    /// Transform attribute access expressions in a single statement
    fn transform_attribute_access_in_stmt(
        &self,
        stmt: &mut Stmt,
        imported_modules: &IndexMap<String, String>,
        bundled_modules: &IndexMap<String, String>,
    ) -> Result<()> {
        match stmt {
            Stmt::Assign(assign) => {
                self.process_assignment_targets(
                    &mut assign.targets,
                    imported_modules,
                    bundled_modules,
                )?;
                self.transform_attribute_access_in_expr(
                    &mut assign.value,
                    imported_modules,
                    bundled_modules,
                )?;
            }
            Stmt::Expr(expr_stmt) => {
                self.transform_attribute_access_in_expr(
                    &mut expr_stmt.value,
                    imported_modules,
                    bundled_modules,
                )?;
            }
            // Add more statement types as needed
            _ => {}
        }
        Ok(())
    }

    /// Determine if an assignment target should be transformed
    /// For relative imports, we generally don't want to transform simple variable names
    fn should_transform_assignment_target(
        &self,
        target: &Expr,
        imported_modules: &IndexMap<String, String>,
    ) -> bool {
        match target {
            Expr::Name(_) => {
                // Simple variable names should not be transformed for relative imports
                // This preserves the original variable name in assignments like "message = messages.message"
                false
            }
            Expr::Attribute(attr) => self.should_transform_attribute_target(attr, imported_modules),
            _ => {
                // Other expression types, transform them
                true
            }
        }
    }

    /// Helper method to check if an attribute target should be transformed
    fn should_transform_attribute_target(
        &self,
        attr: &ast::ExprAttribute,
        imported_modules: &IndexMap<String, String>,
    ) -> bool {
        // For attribute access targets, check if it's a relative import
        if let Expr::Name(name) = attr.value.as_ref() {
            if let Some(module_prefix) = imported_modules.get(name.id.as_str()) {
                // Don't transform targets for relative imports (indicated by REL_IMPORT_SUFFIX)
                !module_prefix.ends_with(REL_IMPORT_SUFFIX)
            } else {
                // Not an imported module, safe to transform
                true
            }
        } else {
            // Complex attribute access, transform it
            true
        }
    }

    /// Transform attribute access expressions in an expression
    fn transform_attribute_access_in_expr(
        &self,
        expr: &mut Expr,
        imported_modules: &IndexMap<String, String>,
        bundled_modules: &IndexMap<String, String>,
    ) -> Result<()> {
        match expr {
            Expr::Attribute(attr) => {
                // Check if this is an attribute access on an imported module
                let Expr::Name(name) = attr.value.as_ref() else {
                    return Ok(());
                };
                let Some(module_prefix) = imported_modules.get(name.id.as_str()) else {
                    return Ok(());
                };

                // Skip transformation if this name has an import alias assignment
                // If there's an import alias, the name becomes a local variable, not a module reference
                if self.import_aliases.contains_key(name.id.as_str()) {
                    return Ok(());
                }

                // Check if this is a relative import (indicated by the REL_IMPORT_SUFFIX)
                let bundled_var_name = if module_prefix.ends_with(REL_IMPORT_SUFFIX) {
                    self.resolve_relative_import_variable(
                        &name.id,
                        (module_prefix, &attr.attr),
                        bundled_modules,
                    )
                } else {
                    // Regular module import transformation
                    format!("__{}_{}", module_prefix, attr.attr)
                };

                log::debug!(
                    "Transforming {}.{} -> {}",
                    name.id,
                    attr.attr,
                    bundled_var_name
                );

                // Replace the entire attribute expression with a simple name
                *expr = Expr::Name(ast::ExprName {
                    id: bundled_var_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                });
            }
            Expr::Call(call) => {
                self.transform_attribute_access_in_expr(
                    &mut call.func,
                    imported_modules,
                    bundled_modules,
                )?;
                for arg in &mut call.arguments.args {
                    self.transform_attribute_access_in_expr(
                        arg,
                        imported_modules,
                        bundled_modules,
                    )?;
                }
                for keyword in &mut call.arguments.keywords {
                    self.transform_attribute_access_in_expr(
                        &mut keyword.value,
                        imported_modules,
                        bundled_modules,
                    )?;
                }
            }
            Expr::BinOp(binop) => {
                self.transform_attribute_access_in_expr(
                    &mut binop.left,
                    imported_modules,
                    bundled_modules,
                )?;
                self.transform_attribute_access_in_expr(
                    &mut binop.right,
                    imported_modules,
                    bundled_modules,
                )?;
            }
            Expr::Compare(compare) => {
                self.transform_attribute_access_in_expr(
                    &mut compare.left,
                    imported_modules,
                    bundled_modules,
                )?;
                for comparator in &mut compare.comparators {
                    self.transform_attribute_access_in_expr(
                        comparator,
                        imported_modules,
                        bundled_modules,
                    )?;
                }
            }
            Expr::List(list) => {
                for elt in &mut list.elts {
                    self.transform_attribute_access_in_expr(
                        elt,
                        imported_modules,
                        bundled_modules,
                    )?;
                }
            }
            Expr::Tuple(tuple) => {
                for elt in &mut tuple.elts {
                    self.transform_attribute_access_in_expr(
                        elt,
                        imported_modules,
                        bundled_modules,
                    )?;
                }
            }
            Expr::Dict(dict) => {
                dict.items.iter_mut().try_for_each(|item| {
                    self.transform_dict_item(item, imported_modules, bundled_modules)
                })?;
                return Ok(());
            }
            Expr::If(if_expr) => {
                self.transform_attribute_access_in_expr(
                    &mut if_expr.test,
                    imported_modules,
                    bundled_modules,
                )?;
                self.transform_attribute_access_in_expr(
                    &mut if_expr.body,
                    imported_modules,
                    bundled_modules,
                )?;
                self.transform_attribute_access_in_expr(
                    &mut if_expr.orelse,
                    imported_modules,
                    bundled_modules,
                )?;
            }
            Expr::UnaryOp(unary) => {
                self.transform_attribute_access_in_expr(
                    &mut unary.operand,
                    imported_modules,
                    bundled_modules,
                )?;
            }
            Expr::Subscript(subscript) => {
                self.transform_attribute_access_in_expr(
                    &mut subscript.value,
                    imported_modules,
                    bundled_modules,
                )?;
                self.transform_attribute_access_in_expr(
                    &mut subscript.slice,
                    imported_modules,
                    bundled_modules,
                )?;
            }
            // Add more expression types as needed
            _ => {}
        }
        Ok(())
    }

    /// Transform dict item expressions
    fn transform_dict_item(
        &self,
        item: &mut ast::DictItem,
        imported_modules: &IndexMap<String, String>,
        bundled_modules: &IndexMap<String, String>,
    ) -> Result<()> {
        if let Some(key) = &mut item.key {
            self.transform_attribute_access_in_expr(key, imported_modules, bundled_modules)?;
        }
        self.transform_attribute_access_in_expr(
            &mut item.value,
            imported_modules,
            bundled_modules,
        )?;
        Ok(())
    }

    /// Resolve relative import variable to actual bundled name
    fn resolve_relative_import_variable(
        &self,
        identifier_name: &str,
        module_attr_pair: (&str, &str), // (module_prefix, attr_name)
        bundled_modules: &IndexMap<String, String>,
    ) -> String {
        let (module_prefix, attr_name) = module_attr_pair;

        // This is a relative import - resolve to the actual bundled variable
        // For "greeting.message" where greeting is from "from . import greeting",
        // we need to handle modules with different import strategies differently

        // Safely remove the REL_IMPORT_SUFFIX using strip_suffix
        let target_module_path_underscore = module_prefix
            .strip_suffix(REL_IMPORT_SUFFIX)
            .unwrap_or(module_prefix);

        // Convert underscores back to dots for strategy lookup
        let target_module_path = target_module_path_underscore
            .cow_replace('_', ".")
            .into_owned();

        log::debug!(
            "Relative import transformation: module_prefix='{}', target_module_path_underscore='{}', target_module_path='{}', attr='{}'",
            module_prefix,
            target_module_path_underscore,
            target_module_path,
            attr_name
        );

        // Check if the target module uses ModuleImport strategy
        // If so, we should access the attribute through the module namespace
        if let Some(import_strategy) = self.import_strategies.get(&target_module_path) {
            if matches!(import_strategy, crate::emit::ImportStrategy::ModuleImport) {
                // For ModuleImport strategy, access the attribute through the module namespace
                let module_attr_access = format!("{}.{}", target_module_path, attr_name);
                log::debug!(
                    "Resolved {}.{} -> {} (ModuleImport strategy)",
                    identifier_name,
                    attr_name,
                    module_attr_access
                );
                return module_attr_access;
            }
        }

        // For FromImport or Dependency strategy, look up the bundled variable name
        let lookup_key = format!("{}.{}", target_module_path, attr_name);
        let actual_bundled_name = bundled_modules
            .get(&lookup_key)
            .cloned()
            .unwrap_or_else(|| {
                // If no bundled name exists, use the original variable name
                // This happens when the variable wasn't renamed due to no conflicts
                log::debug!(
                    "No bundled mapping found for '{}', using original variable name '{}'",
                    lookup_key,
                    attr_name
                );
                attr_name.to_string()
            });

        log::debug!(
            "Resolved {}.{} -> {} (lookup_key: {})",
            identifier_name,
            attr_name,
            actual_bundled_name,
            lookup_key
        );

        actual_bundled_name
    }

    /// Transform module AST to remove import statements that have alias assignments
    pub fn transform_module_ast(&mut self, module_ast: &mut ast::ModModule) -> Result<()> {
        // If we have import aliases, we need to remove the original import statements
        // that have been replaced by alias assignments
        if self.import_aliases.is_empty() {
            log::debug!("No import aliases to transform");
            return Ok(());
        }

        log::debug!(
            "Transforming {} import statements with alias assignments",
            self.import_aliases.len()
        );

        // Collect the modules and aliases that have alias assignments
        let mut aliased_imports = IndexSet::new();
        let mut aliased_from_imports = IndexMap::new();

        for (alias_name, import_alias) in &self.import_aliases {
            if !import_alias.has_explicit_alias {
                continue;
            }
            if import_alias.is_from_import {
                aliased_from_imports
                    .entry(import_alias.module_name.clone())
                    .or_insert_with(IndexSet::new)
                    .insert(alias_name.clone());
                continue;
            }
            // For regular imports, track the module being aliased
            aliased_imports.insert(import_alias.module_name.clone());
        }

        // Filter out import statements that have alias assignments
        let original_body = std::mem::take(&mut module_ast.body);
        module_ast.body = original_body
            .into_iter()
            .filter_map(|stmt| {
                self.filter_import_statement(stmt, &aliased_imports, &aliased_from_imports)
            })
            .collect();

        log::debug!("Import transformation complete");
        Ok(())
    }

    /// Filter individual import statements based on alias assignments
    fn filter_import_statement(
        &self,
        stmt: Stmt,
        aliased_imports: &IndexSet<String>,
        aliased_from_imports: &IndexMap<String, IndexSet<String>>,
    ) -> Option<Stmt> {
        match &stmt {
            Stmt::Import(import_stmt) => {
                // Filter out aliased imports from regular import statements
                let filtered_names: Vec<Alias> = import_stmt
                    .names
                    .iter()
                    .filter(|alias| {
                        let module_name = alias.name.as_str();
                        // Keep the import if it's not aliased OR if it doesn't have an explicit alias
                        !aliased_imports.contains(module_name) || alias.asname.is_none()
                    })
                    .cloned()
                    .collect();

                if filtered_names.is_empty() {
                    // Remove the entire import statement if all imports are aliased
                    return None;
                }
                if filtered_names.len() < import_stmt.names.len() {
                    // Create a new import statement with only non-aliased imports
                    return Some(Stmt::Import(ast::StmtImport {
                        names: filtered_names,
                        range: import_stmt.range,
                    }));
                }
                // Keep the original statement
                Some(stmt)
            }
            Stmt::ImportFrom(import_from_stmt) => {
                let Some(module) = &import_from_stmt.module else {
                    // No module specified, keep the statement
                    return Some(stmt);
                };
                let module_name = module.as_str();

                let Some(aliased_names) = aliased_from_imports.get(module_name) else {
                    // Module not in aliased from imports, keep the statement
                    return Some(stmt);
                };

                // Filter out aliased names from from import statements
                let filtered_names =
                    Self::filter_import_names(aliased_names, &import_from_stmt.names);

                if filtered_names.is_empty() {
                    // Remove the entire from import statement if all imports are aliased
                    return None;
                }
                if filtered_names.len() < import_from_stmt.names.len() {
                    // Create a new from import statement with only non-aliased imports
                    return Some(Stmt::ImportFrom(ast::StmtImportFrom {
                        module: import_from_stmt.module.clone(),
                        names: filtered_names,
                        level: import_from_stmt.level,
                        range: import_from_stmt.range,
                    }));
                }
                // Keep the original statement
                Some(stmt)
            }
            _ => {
                // Not an import statement, keep it
                Some(stmt)
            }
        }
    }

    /// Filters import names by removing those that are aliased
    fn filter_import_names(
        aliased_names: &IndexSet<String>,
        names: &[ast::Alias],
    ) -> Vec<ast::Alias> {
        names
            .iter()
            .filter_map(|alias| {
                let import_name = alias
                    .asname
                    .as_ref()
                    .map(|name| name.as_str())
                    .unwrap_or_else(|| alias.name.as_str());

                // Keep the import if it's not in our aliased names
                if aliased_names.contains(import_name) {
                    None
                } else {
                    Some(alias.clone())
                }
            })
            .collect()
    }

    fn handle_submodule_conflicts(
        &mut self,
        name: &str,
        modules: &[String],
        package_module: &str,
    ) -> bool {
        let submodules: Vec<String> = modules
            .iter()
            .filter(|m| {
                m.as_str() != package_module && m.starts_with(&format!("{}.", package_module))
            })
            .cloned()
            .collect();
        if !submodules.is_empty() {
            self.resolve_submodule_conflicts(name, &submodules);
            return true;
        }
        false
    }

    /// Process relative import with module name (e.g., "from .greeting import message" or "from ..messages import message")
    fn process_relative_module_import(
        &self,
        names: &[Alias],
        context: (&str, &str, u32, &IndexMap<String, String>), // (relative_module_name, current_module_name, level, bundled_modules)
        imported_modules: &mut IndexMap<String, String>,
    ) {
        let (relative_module_name, current_module_name, level, bundled_modules) = context;

        // Resolve the target module path based on the relative import level
        let target_module_path =
            self.resolve_relative_module_path(current_module_name, relative_module_name, level);

        log::debug!(
            "Resolving relative import: from {}{} import [...] (level={}) in module '{}' -> target module: '{}'",
            ".".repeat(level as usize),
            relative_module_name,
            level,
            current_module_name,
            target_module_path
        );

        for alias in names {
            let imported_name = alias.name.as_str();

            // Resolve the bundled name using import strategy and bundled mappings
            let bundled_name = self.resolve_relative_import_name(
                &target_module_path,
                imported_name,
                bundled_modules,
            );

            let lookup_key = format!("{}.{}", target_module_path, imported_name);
            log::debug!(
                "Transforming relative import '{}' from '{}' -> bundled variable '{}'",
                imported_name,
                lookup_key,
                bundled_name
            );

            imported_modules.insert(imported_name.to_string(), bundled_name);
        }
    }

    /// Helper to return a namespace reference if ModuleImport strategy applies
    fn module_import_namespace_ref(
        &self,
        target_module_path: &str,
        imported_name: &str,
    ) -> Option<String> {
        if let Some(strategy) = self.import_strategies.get(target_module_path) {
            if matches!(strategy, crate::emit::ImportStrategy::ModuleImport) {
                let ns_ref = format!("{}.{}", target_module_path, imported_name);
                log::debug!(
                    "Target module '{}' uses ModuleImport strategy, using namespace reference '{}'",
                    target_module_path,
                    ns_ref
                );
                return Some(ns_ref);
            }
        }
        None
    }

    /// Resolve the bundled name for a relative import considering import strategies
    fn resolve_relative_import_name(
        &self,
        target_module_path: &str,
        imported_name: &str,
        bundled_modules: &IndexMap<String, String>,
    ) -> String {
        // Use helper to check ModuleImport strategy first
        if let Some(ns_ref) = self.module_import_namespace_ref(target_module_path, imported_name) {
            return ns_ref;
        }

        // Look for the bundled variable name for this import
        let lookup_key = format!("{}.{}", target_module_path, imported_name);
        bundled_modules
            .get(&lookup_key)
            .cloned()
            .unwrap_or_else(|| {
                self.resolve_relative_import_fallback(
                    target_module_path,
                    imported_name,
                    &lookup_key,
                )
            })
    }

    /// Resolve fallback for relative import when no explicit mapping is found
    fn resolve_relative_import_fallback(
        &self,
        target_module_path: &str,
        imported_name: &str,
        lookup_key: &str,
    ) -> String {
        // Use helper to check ModuleImport strategy
        if let Some(ns_ref) = self.module_import_namespace_ref(target_module_path, imported_name) {
            return ns_ref;
        }

        // For variables without explicit mapping, assume they become global variables
        // with their original name (this is the case for non-conflicted variables)
        log::debug!(
            "No bundled mapping found for '{}', assuming global variable '{}'",
            lookup_key,
            imported_name
        );
        imported_name.to_string()
    }

    /// Resolve relative module path based on current module and import level
    /// The level represents the number of dots in the import:
    /// - level=1: "from . import" (current package)
    /// - level=2: "from .. import" (parent package)
    /// - level=3: "from ... import" (grandparent package)
    ///
    /// Examples:
    /// - current_module="greetings.greeting", relative_module="messages", level=2 ("from ..messages")
    ///   -> resolves to "greetings.messages"
    /// - current_module="greetings.greeting", relative_module="submodule", level=1 ("from .submodule")
    ///   -> resolves to "greetings.greeting.submodule"
    fn resolve_relative_module_path(
        &self,
        current_module: &str,
        relative_module: &str,
        level: u32,
    ) -> String {
        let current_parts: Vec<&str> = current_module.split('.').collect();

        // Calculate target package depth based on level
        // level=1 means current package, level=2 means parent package, etc.
        // So we need to go up (level-1) package levels
        let levels_up = (level as usize).saturating_sub(1);
        let target_depth = current_parts.len().saturating_sub(levels_up);

        // Build the target module path
        let mut target_parts = current_parts[..target_depth].to_vec();
        if !relative_module.is_empty() {
            target_parts.push(relative_module);
        }

        let result = target_parts.join(".");

        log::debug!(
            "resolve_relative_module_path: current='{}', relative='{}', level={} -> levels_up={}, target_depth={}, result='{}'",
            current_module,
            relative_module,
            level,
            levels_up,
            target_depth,
            result
        );

        result
    }

    /// Process relative dot import (e.g., "from . import module")
    fn process_relative_dot_import(
        &self,
        names: &[Alias],
        context: (&str, &IndexMap<String, String>), // (module_name, bundled_modules)
        imported_modules: &mut IndexMap<String, String>,
    ) {
        let (module_name, bundled_modules) = context;

        for alias in names {
            let imported_name = alias.name.as_str();

            // For "from . import module", we need to resolve to the sibling module
            // in the same package. This means we should construct the full module path
            // by combining the current module's package with the imported name
            let target_module_path = if let Some(last_dot) = module_name.rfind('.') {
                // We're in a subpackage, so the target is in the same subpackage
                let parent_package = &module_name[..last_dot];
                format!("{}.{}", parent_package, imported_name)
            } else {
                // We're at the top level, so the target is in the same top-level package
                format!("{}.{}", module_name, imported_name)
            };

            log::debug!(
                "Resolving relative dot import: from . import {} in module '{}' -> target module: '{}'",
                imported_name,
                module_name,
                target_module_path
            );

            // Look for the bundled variable name for this import
            // For a module import like "from . import messages", we need to track this
            // so that attribute access like "messages.message" can be transformed
            // directly to the bundled variable "__greetings_messages_message"
            let module_alias = if let Some(bundled_name) = bundled_modules.get(&target_module_path)
            {
                // Use the actual bundled module name from the mapping
                bundled_name.clone()
            } else {
                // Fallback to the old behavior if not found in bundled_modules
                format!(
                    "{}{}",
                    target_module_path.cow_replace('.', "_"),
                    REL_IMPORT_SUFFIX
                )
            };

            log::debug!(
                "Mapping relative dot import: {} -> {} (target module: {})",
                imported_name,
                module_alias,
                target_module_path
            );

            imported_modules.insert(imported_name.to_string(), module_alias);
        }
    }

    /// Check if a name was renamed due to a conflict in the specified module
    pub fn is_conflict_based_rename(&self, original_name: &str, module_name: &str) -> bool {
        if let Some(conflict) = self.name_conflicts.get(original_name) {
            conflict.modules.contains(&module_name.to_string())
        } else {
            false
        }
    }

    /// Resolve alias name for inlined modules (FromImport/Dependency)
    fn resolve_alias_name_for_inlined_module(&self, import_alias: &ImportAlias) -> String {
        if let Some(conflict) = self.name_conflicts.get(&import_alias.original_name) {
            // If there's a conflict, find the renamed version for the source module
            if let Some(renamed) = conflict.renamed_versions.get(&import_alias.module_name) {
                renamed.clone()
            } else {
                // Fallback to original name if not found in conflict mapping
                import_alias.original_name.clone()
            }
        } else {
            // No conflict, use the original name
            import_alias.original_name.clone()
        }
    }

    /// Get all module-level symbols for a given module
    /// Returns a map of original_name -> final_name (renamed or original)
    pub fn get_module_symbols(&self, module_name: &str) -> IndexMap<String, String> {
        let mut symbols = IndexMap::new();

        // Collect all module-level symbols from the symbol table
        let module_prefix = format!("{}::", module_name);
        for (symbol_key, symbol) in &self.symbols {
            if symbol_key.starts_with(&module_prefix) && symbol.is_global && !symbol.is_imported {
                let original_name = &symbol.name;

                // Get the final name (renamed or original)
                let final_name = self.get_final_symbol_name(module_name, original_name);
                symbols.insert(original_name.clone(), final_name);
            }
        }

        symbols
    }

    /// Get the final name for a symbol (renamed or original)
    fn get_final_symbol_name(&self, module_name: &str, original_name: &str) -> String {
        if let Some(renames) = self.module_renames.get(module_name) {
            renames
                .get(original_name)
                .cloned()
                .unwrap_or_else(|| original_name.to_string())
        } else {
            original_name.to_string()
        }
    }
}

impl Default for AstRewriter {
    fn default() -> Self {
        Self::new(10) // Default to Python 3.10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_detection_with_ruff() {
        // Test that ruff_python_stdlib::keyword::is_keyword works as expected
        let known_keywords = [
            "def", "class", "if", "else", "for", "while", "import", "from", "False", "None",
            "True", "and", "as", "assert", "async", "await",
        ];
        for &keyword_str in &known_keywords {
            assert!(
                keyword::is_keyword(keyword_str),
                "Known keyword '{}' is not recognized by ruff_python_stdlib::keyword::is_keyword",
                keyword_str
            );
        }

        // Test a few known non-keywords to ensure the function works correctly
        let non_keywords = ["hello", "world", "foo", "bar", "variable"];
        for &non_keyword in &non_keywords {
            assert!(
                !keyword::is_keyword(non_keyword),
                "Non-keyword '{}' was incorrectly identified as a keyword by ruff_python_stdlib::keyword::is_keyword",
                non_keyword
            );
        }
    }

    #[test]
    fn test_is_reserved_name_functionality() {
        let ast_rewriter = AstRewriter::new(10); // Python 3.10

        // Test that keywords are detected as reserved
        let keywords = ["def", "class", "if", "for", "import"];
        for &keyword_str in &keywords {
            assert!(
                ast_rewriter.is_reserved_name(keyword_str),
                "Keyword '{}' should be detected as reserved",
                keyword_str
            );
        }

        // Test that builtins are detected as reserved
        let builtins_sample = ["len", "str", "int", "list"];
        for &builtin in &builtins_sample {
            assert!(
                ast_rewriter.is_reserved_name(builtin),
                "Builtin '{}' should be detected as reserved",
                builtin
            );
        }

        // Test that regular names are not reserved
        let regular_names = ["my_variable", "foo", "bar"];
        for &name in &regular_names {
            assert!(
                !ast_rewriter.is_reserved_name(name),
                "Regular name '{}' should not be detected as reserved",
                name
            );
        }
    }

    #[test]
    fn test_process_relative_dot_import() {
        use ruff_text_size::TextRange;

        let ast_rewriter = AstRewriter::new(10);

        // Create test aliases for "from . import greeting, helpers"
        let aliases = vec![
            Alias {
                name: Identifier::new("greeting", TextRange::default()),
                asname: None,
                range: TextRange::default(),
            },
            Alias {
                name: Identifier::new("helpers", TextRange::default()),
                asname: None,
                range: TextRange::default(),
            },
        ];

        // Create bundled modules mapping
        let mut bundled_modules = IndexMap::new();
        bundled_modules.insert(
            "greetings.greeting".to_string(),
            "greetings_greeting".to_string(),
        );
        bundled_modules.insert(
            "greetings.helpers".to_string(),
            "greetings_helpers".to_string(),
        );

        let mut imported_modules = IndexMap::new();

        // Test the process_relative_dot_import method
        ast_rewriter.process_relative_dot_import(
            &aliases,
            ("greetings", &bundled_modules),
            &mut imported_modules,
        );

        // Verify the imported modules were processed correctly
        assert_eq!(imported_modules.len(), 2);
        assert_eq!(
            imported_modules.get("greeting"),
            Some(&"greetings_greeting".to_string())
        );
        assert_eq!(
            imported_modules.get("helpers"),
            Some(&"greetings_helpers".to_string())
        );
    }
}
