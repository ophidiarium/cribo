use anyhow::Result;
use cow_utils::CowUtils;
use ruff_python_ast::{self as ast, Alias, Expr, ExprContext, Identifier, Stmt};
use ruff_python_stdlib::{builtins, keyword};
use ruff_text_size::TextRange;
use std::collections::{HashMap, HashSet};

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
    pub renamed_versions: HashMap<String, String>,
}

/// AST rewriter for handling import aliases and name conflicts
pub struct AstRewriter {
    /// Map of import aliases that need to be resolved in the entry module
    import_aliases: HashMap<String, ImportAlias>,
    /// Map of name conflicts and their resolutions
    name_conflicts: HashMap<String, NameConflict>,
    /// Map of renamed identifiers per module
    module_renames: HashMap<String, HashMap<String, String>>,
    /// Set of all reserved names (builtins, keywords, already used names)
    reserved_names: HashSet<String>,
    /// Symbol table for comprehensive scope analysis
    symbols: HashMap<String, Symbol>,
    /// Python version for builtin checks
    python_version: u8,
}

impl AstRewriter {
    pub fn new(python_version: u8) -> Self {
        // Initialize reserved names with Python builtins and keywords using ruff_python_stdlib
        let mut reserved_names = HashSet::new();

        // Add all Python built-ins for the specified version
        for builtin in builtins::python_builtins(python_version, false) {
            reserved_names.insert(builtin.to_string());
        }

        // Note: Python keywords are checked dynamically using ruff_python_stdlib::keyword::is_keyword
        // rather than pre-populating the set, for better maintainability

        Self {
            import_aliases: HashMap::new(),
            name_conflicts: HashMap::new(),
            module_renames: HashMap::new(),
            reserved_names,
            symbols: HashMap::new(),
            python_version,
        }
    }

    /// Public getter for import_aliases (for testing)
    pub fn import_aliases(&self) -> &HashMap<String, ImportAlias> {
        &self.import_aliases
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
                    original_name: alias.name.to_string(),
                    alias_name: asname.to_string(),
                    module_name: module.to_string(),
                    is_from_import: true,
                    has_explicit_alias: true,
                    is_module_import: false, // Will be set later based on resolver
                }
            } else {
                // Import without alias: from module import name
                ImportAlias {
                    original_name: alias.name.to_string(),
                    alias_name: alias.name.to_string(), // Same as original name
                    module_name: module.to_string(),
                    is_from_import: true,
                    has_explicit_alias: false,
                    is_module_import: false, // Will be set later based on resolver
                }
            };

            let key = if alias.asname.is_some() {
                import_alias.alias_name.clone()
            } else {
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
                    original_name: alias.name.to_string(),
                    alias_name: asname.to_string(),
                    module_name: alias.name.to_string(),
                    is_from_import: false,
                    has_explicit_alias: true,
                    is_module_import: true, // Regular imports are always module imports
                };
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
                    name: func_def.name.to_string(),
                    scope_type: scope_type.clone(),
                    is_parameter: false,
                    is_global: matches!(scope_type, ScopeType::Module),
                    is_nonlocal: false,
                    is_imported: false,
                    definitions: vec![module_name.to_string()],
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
                    name: class_def.name.to_string(),
                    scope_type: scope_type.clone(),
                    is_parameter: false,
                    is_global: matches!(scope_type, ScopeType::Module),
                    is_nonlocal: false,
                    is_imported: false,
                    definitions: vec![module_name.to_string()],
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
                        name: name.id.to_string(),
                        scope_type: scope_type.clone(),
                        is_parameter: false,
                        is_global: true,
                        is_nonlocal: false,
                        is_imported: false,
                        definitions: vec![module_name.to_string()],
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
        // First collect all symbols
        self.collect_symbols(modules);

        let name_to_modules = self.collect_module_level_identifiers(modules);
        self.generate_conflict_resolutions(name_to_modules);
    }

    /// Collect all module-level identifiers that could potentially conflict
    fn collect_module_level_identifiers(
        &self,
        modules: &[(String, &ast::ModModule)],
    ) -> HashMap<String, Vec<String>> {
        let mut name_to_modules: HashMap<String, Vec<String>> = HashMap::new();

        for (module_name, _) in modules {
            self.collect_symbols_for_module(module_name, &mut name_to_modules);
        }

        name_to_modules
    }

    /// Collect symbols for a specific module
    fn collect_symbols_for_module(
        &self,
        module_name: &str,
        name_to_modules: &mut HashMap<String, Vec<String>>,
    ) {
        for (symbol_key, symbol) in &self.symbols {
            if self.is_conflictable_symbol(symbol_key, symbol, module_name) {
                name_to_modules
                    .entry(symbol.name.clone())
                    .or_default()
                    .push(module_name.to_string());
            }
        }
    }

    /// Check if a symbol can potentially conflict
    fn is_conflictable_symbol(&self, symbol_key: &str, symbol: &Symbol, module_name: &str) -> bool {
        symbol_key.starts_with(&format!("{}::", module_name))
            && symbol.is_global
            && !symbol.is_imported
    }

    /// Generate conflict resolutions for conflicting names
    fn generate_conflict_resolutions(&mut self, name_to_modules: HashMap<String, Vec<String>>) {
        for (name, modules) in name_to_modules {
            if modules.len() > 1 {
                self.resolve_name_conflict(&name, &modules);
            }
        }
    }

    /// Resolve a specific name conflict
    fn resolve_name_conflict(&mut self, name: &str, modules: &[String]) {
        let mut renamed_versions = HashMap::new();

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
        self.name_conflicts.insert(name.to_string(), conflict);
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
        if let Some(renames) = self.module_renames.get(module_name) {
            self.apply_renames_to_ast(&mut module_ast.body, renames)?;
        }
        Ok(())
    }

    /// Apply renames to an AST node recursively
    fn apply_renames_to_ast(
        &self,
        statements: &mut Vec<Stmt>,
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
    ) -> Result<()> {
        for target in &mut assign.targets {
            self.apply_renames_to_expr(target, renames)?;
        }
        self.apply_renames_to_expr(&mut assign.value, renames)
    }

    /// Apply renames to return statement
    fn apply_renames_to_return(
        &self,
        return_stmt: &mut ast::StmtReturn,
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut if_stmt.test, renames)?;
        self.apply_renames_to_ast(&mut if_stmt.body, renames)?;

        for clause in &mut if_stmt.elif_else_clauses {
            if let Some(test) = &mut clause.test {
                self.apply_renames_to_expr(test, renames)?;
            }
            self.apply_renames_to_ast(&mut clause.body, renames)?;
        }
        Ok(())
    }

    /// Apply renames to while statement
    fn apply_renames_to_while(
        &self,
        while_stmt: &mut ast::StmtWhile,
        renames: &HashMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut while_stmt.test, renames)?;
        self.apply_renames_to_ast(&mut while_stmt.body, renames)?;
        self.apply_renames_to_ast(&mut while_stmt.orelse, renames)
    }

    /// Apply renames to for statement
    fn apply_renames_to_for(
        &self,
        for_stmt: &mut ast::StmtFor,
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut aug_assign.target, renames)?;
        self.apply_renames_to_expr(&mut aug_assign.value, renames)
    }

    /// Apply renames to annotated assignment statement
    fn apply_renames_to_ann_assign(
        &self,
        ann_assign: &mut ast::StmtAnnAssign,
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
    ) -> Result<()> {
        match expr {
            Expr::Name(name) => self.apply_renames_to_name(name, renames),
            Expr::Call(call) => self.apply_renames_to_call(call, renames),
            Expr::Attribute(attr) => self.apply_renames_to_expr(&mut attr.value, renames),
            Expr::BinOp(binop) => self.apply_renames_to_binop(binop, renames),
            Expr::UnaryOp(unary) => self.apply_renames_to_expr(&mut unary.operand, renames),
            Expr::Compare(compare) => self.apply_renames_to_compare(compare, renames),
            Expr::List(list) => self.apply_renames_to_list(list, renames),
            Expr::Dict(dict) => self.apply_renames_to_dict(dict, renames),
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
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_expr(&mut binop.left, renames)?;
        self.apply_renames_to_expr(&mut binop.right, renames)
    }

    /// Apply renames to a comparison expression
    fn apply_renames_to_compare(
        &self,
        compare: &mut ast::ExprCompare,
        renames: &HashMap<String, String>,
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
        renames: &HashMap<String, String>,
    ) -> Result<()> {
        self.apply_renames_to_collection(&mut list.elts, renames)
    }

    /// Apply renames to a dictionary expression
    fn apply_renames_to_dict(
        &self,
        dict: &mut ast::ExprDict,
        renames: &HashMap<String, String>,
    ) -> Result<()> {
        for item in &mut dict.items {
            if let Some(key) = &mut item.key {
                self.apply_renames_to_expr(key, renames)?;
            }
            self.apply_renames_to_expr(&mut item.value, renames)?;
        }
        Ok(())
    }

    /// Apply renames to a collection of expressions
    fn apply_renames_to_collection(
        &self,
        elts: &mut [Expr],
        renames: &HashMap<String, String>,
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

        // Only generate assignment if there's an explicit alias or a name conflict
        if import_alias.has_explicit_alias || has_conflict {
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

    /// Resolve the actual name for an import considering name conflicts
    fn resolve_actual_name_for_conflict(&self, import_alias: &ImportAlias) -> String {
        if let Some(conflict) = self.name_conflicts.get(&import_alias.original_name) {
            conflict
                .renamed_versions
                .get(&import_alias.module_name)
                .cloned()
                .unwrap_or_else(|| import_alias.original_name.clone())
        } else {
            // For from imports, check if this is a module import
            if import_alias.is_from_import && import_alias.is_module_import {
                // This is a module import (e.g., from greetings import greeting)
                // Use the full module path (e.g., greetings.greeting)
                format!(
                    "{}.{}",
                    import_alias.module_name, import_alias.original_name
                )
            } else {
                // This is a value import or regular import
                // Use the original name directly
                import_alias.original_name.clone()
            }
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
        let mut aliased_imports = HashSet::new();
        let mut aliased_from_imports = HashMap::new();

        for (alias_name, import_alias) in &self.import_aliases {
            if import_alias.has_explicit_alias {
                if import_alias.is_from_import {
                    // For from imports, track module -> alias mappings
                    aliased_from_imports
                        .entry(import_alias.module_name.clone())
                        .or_insert_with(HashSet::new)
                        .insert(alias_name.clone());
                } else {
                    // For regular imports, track the module being aliased
                    aliased_imports.insert(import_alias.module_name.clone());
                }
            }
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
        aliased_imports: &HashSet<String>,
        aliased_from_imports: &HashMap<String, HashSet<String>>,
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
                    None
                } else if filtered_names.len() < import_stmt.names.len() {
                    // Create a new import statement with only non-aliased imports
                    Some(Stmt::Import(ast::StmtImport {
                        names: filtered_names,
                        range: import_stmt.range,
                    }))
                } else {
                    // Keep the original statement
                    Some(stmt)
                }
            }
            Stmt::ImportFrom(import_from_stmt) => {
                if let Some(module) = &import_from_stmt.module {
                    let module_name = module.as_str();

                    if let Some(aliased_names) = aliased_from_imports.get(module_name) {
                        // Filter out aliased names from from import statements
                        let filtered_names: Vec<Alias> = import_from_stmt
                            .names
                            .iter()
                            .filter(|alias| {
                                let import_name = if let Some(asname) = &alias.asname {
                                    asname.as_str()
                                } else {
                                    alias.name.as_str()
                                };
                                // Keep the import if it's not in our aliased names
                                !aliased_names.contains(import_name)
                            })
                            .cloned()
                            .collect();

                        if filtered_names.is_empty() {
                            // Remove the entire from import statement if all imports are aliased
                            None
                        } else if filtered_names.len() < import_from_stmt.names.len() {
                            // Create a new from import statement with only non-aliased imports
                            Some(Stmt::ImportFrom(ast::StmtImportFrom {
                                module: import_from_stmt.module.clone(),
                                names: filtered_names,
                                level: import_from_stmt.level,
                                range: import_from_stmt.range,
                            }))
                        } else {
                            // Keep the original statement
                            Some(stmt)
                        }
                    } else {
                        // Module not in aliased from imports, keep the statement
                        Some(stmt)
                    }
                } else {
                    // No module specified, keep the statement
                    Some(stmt)
                }
            }
            _ => {
                // Not an import statement, keep it
                Some(stmt)
            }
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
}
