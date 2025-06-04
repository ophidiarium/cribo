use anyhow::Result;
use rustpython_parser::ast::{self, Expr, ExprContext, Stmt};
use std::collections::{HashMap, HashSet};

/// Python built-in names that should not be renamed
static PYTHON_BUILTINS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "any",
    "anext",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "__import__",
    "__name__",
    "__doc__",
    "__package__",
    "__loader__",
    "__spec__",
    "__file__",
    "__cached__",
    "__builtins__",
    "True",
    "False",
    "None",
    "NotImplemented",
    "Ellipsis",
    "__debug__",
    "copyright",
    "credits",
    "license",
    "quit",
    "exit",
    "__all__",
];

/// Python keywords that should not be used as identifiers
static PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

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
}

impl AstRewriter {
    pub fn new() -> Self {
        // Initialize reserved names with Python builtins and keywords
        let mut reserved_names = HashSet::new();
        reserved_names.extend(PYTHON_BUILTINS.iter().map(|&s| s.to_string()));
        reserved_names.extend(PYTHON_KEYWORDS.iter().map(|&s| s.to_string()));

        Self {
            import_aliases: HashMap::new(),
            name_conflicts: HashMap::new(),
            module_renames: HashMap::new(),
            reserved_names,
            symbols: HashMap::new(),
        }
    }

    /// Collect import aliases from the entry module before they are removed
    pub fn collect_import_aliases(&mut self, entry_ast: &ast::ModModule, _entry_module_name: &str) {
        for stmt in &entry_ast.body {
            match stmt {
                Stmt::ImportFrom(import_from) => {
                    if let Some(module) = &import_from.module {
                        for alias in &import_from.names {
                            if let Some(asname) = &alias.asname {
                                // Import with explicit alias: from module import name as alias
                                let import_alias = ImportAlias {
                                    original_name: alias.name.to_string(),
                                    alias_name: asname.to_string(),
                                    module_name: module.to_string(),
                                    is_from_import: true,
                                    has_explicit_alias: true,
                                };
                                self.import_aliases.insert(asname.to_string(), import_alias);
                            } else {
                                // Import without alias: from module import name
                                let import_alias = ImportAlias {
                                    original_name: alias.name.to_string(),
                                    alias_name: alias.name.to_string(), // Same as original name
                                    module_name: module.to_string(),
                                    is_from_import: true,
                                    has_explicit_alias: false,
                                };
                                self.import_aliases
                                    .insert(alias.name.to_string(), import_alias);
                            }
                        }
                    }
                }
                Stmt::Import(import) => {
                    for alias in &import.names {
                        if let Some(asname) = &alias.asname {
                            let import_alias = ImportAlias {
                                original_name: alias.name.to_string(),
                                alias_name: asname.to_string(),
                                module_name: alias.name.to_string(),
                                is_from_import: false,
                                has_explicit_alias: true,
                            };
                            self.import_aliases.insert(asname.to_string(), import_alias);
                        }
                    }
                }
                _ => {}
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
    fn collect_symbols_from_expr(
        &mut self,
        module_name: &str,
        expr: &Expr,
        scope_type: &ScopeType,
        is_assignment: bool,
    ) {
        match expr {
            Expr::Name(name) => {
                // Skip built-ins
                if PYTHON_BUILTINS.contains(&name.id.as_str()) {
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
                for arg in &call.args {
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

        let mut name_to_modules: HashMap<String, Vec<String>> = HashMap::new();

        // Collect all module-level identifiers (only those that could conflict)
        for (module_name, _) in modules {
            for (symbol_key, symbol) in &self.symbols {
                if symbol_key.starts_with(&format!("{}::", module_name))
                    && symbol.is_global
                    && !symbol.is_imported
                {
                    name_to_modules
                        .entry(symbol.name.clone())
                        .or_default()
                        .push(module_name.clone());
                }
            }
        }

        // Find conflicts and generate unique names
        for (name, modules) in name_to_modules {
            if modules.len() > 1 {
                let mut renamed_versions = HashMap::new();
                for module in &modules {
                    let renamed = self.generate_unique_name(&name, module);
                    renamed_versions.insert(module.clone(), renamed.clone());

                    // Track renames for this module
                    self.module_renames
                        .entry(module.clone())
                        .or_default()
                        .insert(name.clone(), renamed);
                }

                let conflict = NameConflict {
                    name: name.clone(),
                    modules: modules.clone(),
                    renamed_versions,
                };
                self.name_conflicts.insert(name, conflict);
            }
        }
    }

    /// Generate a unique name for a conflicting identifier
    fn generate_unique_name(&mut self, original_name: &str, module_name: &str) -> String {
        // Clean up module name for use as prefix
        let module_prefix = module_name
            .replace(".", "_")
            .replace("-", "_")
            .replace("/", "_");

        let mut counter = 0;

        loop {
            let candidate = if counter == 0 {
                format!("__{}_{}", module_prefix, original_name)
            } else {
                format!("__{}_{}_{}", module_prefix, original_name, counter)
            };

            // Check if the name is available
            if !self.reserved_names.contains(&candidate)
                && !self.is_name_used_in_any_module(&candidate)
            {
                // Reserve the name
                self.reserved_names.insert(candidate.clone());
                return candidate;
            }

            counter += 1;
        }
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

    /// Apply renames to a single statement
    fn apply_renames_to_stmt(
        &self,
        stmt: &mut Stmt,
        renames: &HashMap<String, String>,
    ) -> Result<()> {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                // Rename function definition
                if let Some(new_name) = renames.get(&func_def.name.to_string()) {
                    func_def.name = new_name.clone().into();
                }
                // Rename in return type annotation
                if let Some(returns) = &mut func_def.returns {
                    self.apply_renames_to_expr(returns, renames)?;
                }
                // Rename within function body
                self.apply_renames_to_ast(&mut func_def.body, renames)?;
            }
            Stmt::ClassDef(class_def) => {
                // Rename class definition
                if let Some(new_name) = renames.get(&class_def.name.to_string()) {
                    class_def.name = new_name.clone().into();
                }
                // Rename within class body
                self.apply_renames_to_ast(&mut class_def.body, renames)?;
            }
            Stmt::Assign(assign) => {
                // Rename assignment targets
                for target in &mut assign.targets {
                    self.apply_renames_to_expr(target, renames)?;
                }
                // Rename in assignment value
                self.apply_renames_to_expr(&mut assign.value, renames)?;
            }
            Stmt::Expr(expr_stmt) => {
                self.apply_renames_to_expr(&mut expr_stmt.value, renames)?;
            }
            Stmt::Return(return_stmt) => {
                if let Some(value) = &mut return_stmt.value {
                    self.apply_renames_to_expr(value, renames)?;
                }
            }
            Stmt::If(if_stmt) => {
                self.apply_renames_to_expr(&mut if_stmt.test, renames)?;
                self.apply_renames_to_ast(&mut if_stmt.body, renames)?;
                self.apply_renames_to_ast(&mut if_stmt.orelse, renames)?;
            }
            Stmt::While(while_stmt) => {
                self.apply_renames_to_expr(&mut while_stmt.test, renames)?;
                self.apply_renames_to_ast(&mut while_stmt.body, renames)?;
                self.apply_renames_to_ast(&mut while_stmt.orelse, renames)?;
            }
            Stmt::For(for_stmt) => {
                self.apply_renames_to_expr(&mut for_stmt.target, renames)?;
                self.apply_renames_to_expr(&mut for_stmt.iter, renames)?;
                self.apply_renames_to_ast(&mut for_stmt.body, renames)?;
                self.apply_renames_to_ast(&mut for_stmt.orelse, renames)?;
            }
            Stmt::With(with_stmt) => {
                for item in &mut with_stmt.items {
                    self.apply_renames_to_expr(&mut item.context_expr, renames)?;
                    if let Some(optional_vars) = &mut item.optional_vars {
                        self.apply_renames_to_expr(optional_vars, renames)?;
                    }
                }
                self.apply_renames_to_ast(&mut with_stmt.body, renames)?;
            }
            Stmt::Try(try_stmt) => {
                self.apply_renames_to_ast(&mut try_stmt.body, renames)?;
                for handler in &mut try_stmt.handlers {
                    match handler {
                        ast::ExceptHandler::ExceptHandler(except_data) => {
                            if let Some(type_) = &mut except_data.type_ {
                                self.apply_renames_to_expr(type_, renames)?;
                            }
                            if let Some(name) = &mut except_data.name {
                                if let Some(new_name) = renames.get(name.as_str()) {
                                    *name = new_name.clone().into();
                                }
                            }
                            self.apply_renames_to_ast(&mut except_data.body, renames)?;
                        }
                    }
                }
                self.apply_renames_to_ast(&mut try_stmt.orelse, renames)?;
                self.apply_renames_to_ast(&mut try_stmt.finalbody, renames)?;
            }
            Stmt::AugAssign(aug_assign) => {
                self.apply_renames_to_expr(&mut aug_assign.target, renames)?;
                self.apply_renames_to_expr(&mut aug_assign.value, renames)?;
            }
            Stmt::AnnAssign(ann_assign) => {
                self.apply_renames_to_expr(&mut ann_assign.target, renames)?;
                self.apply_renames_to_expr(&mut ann_assign.annotation, renames)?;
                if let Some(value) = &mut ann_assign.value {
                    self.apply_renames_to_expr(value, renames)?;
                }
            }
            // Add more statement types as needed
            _ => {}
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
            Expr::Name(name) => {
                // Rename variables in both Load and Store contexts
                if let Some(new_name) = renames.get(&name.id.to_string()) {
                    name.id = new_name.clone().into();
                }
            }
            Expr::Call(call) => {
                self.apply_renames_to_expr(&mut call.func, renames)?;
                for arg in &mut call.args {
                    self.apply_renames_to_expr(arg, renames)?;
                }
                for keyword in &mut call.keywords {
                    self.apply_renames_to_expr(&mut keyword.value, renames)?;
                }
            }
            Expr::Attribute(attr) => {
                self.apply_renames_to_expr(&mut attr.value, renames)?;
            }
            Expr::BinOp(binop) => {
                self.apply_renames_to_expr(&mut binop.left, renames)?;
                self.apply_renames_to_expr(&mut binop.right, renames)?;
            }
            Expr::UnaryOp(unary) => {
                self.apply_renames_to_expr(&mut unary.operand, renames)?;
            }
            Expr::Compare(compare) => {
                self.apply_renames_to_expr(&mut compare.left, renames)?;
                for comparator in &mut compare.comparators {
                    self.apply_renames_to_expr(comparator, renames)?;
                }
            }
            Expr::List(list) => {
                for elt in &mut list.elts {
                    self.apply_renames_to_expr(elt, renames)?;
                }
            }
            Expr::Dict(dict) => {
                for key in &mut dict.keys {
                    if let Some(key) = key {
                        self.apply_renames_to_expr(key, renames)?;
                    }
                }
                for value in &mut dict.values {
                    self.apply_renames_to_expr(value, renames)?;
                }
            }
            Expr::Set(set) => {
                for elt in &mut set.elts {
                    self.apply_renames_to_expr(elt, renames)?;
                }
            }
            Expr::Tuple(tuple) => {
                for elt in &mut tuple.elts {
                    self.apply_renames_to_expr(elt, renames)?;
                }
            }
            Expr::BoolOp(bool_op) => {
                for value in &mut bool_op.values {
                    self.apply_renames_to_expr(value, renames)?;
                }
            }
            Expr::IfExp(if_exp) => {
                self.apply_renames_to_expr(&mut if_exp.test, renames)?;
                self.apply_renames_to_expr(&mut if_exp.body, renames)?;
                self.apply_renames_to_expr(&mut if_exp.orelse, renames)?;
            }
            Expr::ListComp(list_comp) => {
                self.apply_renames_to_expr(&mut list_comp.elt, renames)?;
                for generator in &mut list_comp.generators {
                    self.apply_renames_to_expr(&mut generator.target, renames)?;
                    self.apply_renames_to_expr(&mut generator.iter, renames)?;
                    for if_ in &mut generator.ifs {
                        self.apply_renames_to_expr(if_, renames)?;
                    }
                }
            }
            Expr::SetComp(set_comp) => {
                self.apply_renames_to_expr(&mut set_comp.elt, renames)?;
                for generator in &mut set_comp.generators {
                    self.apply_renames_to_expr(&mut generator.target, renames)?;
                    self.apply_renames_to_expr(&mut generator.iter, renames)?;
                    for if_ in &mut generator.ifs {
                        self.apply_renames_to_expr(if_, renames)?;
                    }
                }
            }
            Expr::DictComp(dict_comp) => {
                self.apply_renames_to_expr(&mut dict_comp.key, renames)?;
                self.apply_renames_to_expr(&mut dict_comp.value, renames)?;
                for generator in &mut dict_comp.generators {
                    self.apply_renames_to_expr(&mut generator.target, renames)?;
                    self.apply_renames_to_expr(&mut generator.iter, renames)?;
                    for if_ in &mut generator.ifs {
                        self.apply_renames_to_expr(if_, renames)?;
                    }
                }
            }
            Expr::GeneratorExp(gen_exp) => {
                self.apply_renames_to_expr(&mut gen_exp.elt, renames)?;
                for generator in &mut gen_exp.generators {
                    self.apply_renames_to_expr(&mut generator.target, renames)?;
                    self.apply_renames_to_expr(&mut generator.iter, renames)?;
                    for if_ in &mut generator.ifs {
                        self.apply_renames_to_expr(if_, renames)?;
                    }
                }
            }
            Expr::Subscript(subscript) => {
                self.apply_renames_to_expr(&mut subscript.value, renames)?;
                self.apply_renames_to_expr(&mut subscript.slice, renames)?;
            }
            Expr::Starred(starred) => {
                self.apply_renames_to_expr(&mut starred.value, renames)?;
            }
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
            }
            // Add more expression types as needed
            _ => {}
        }
        Ok(())
    }

    /// Generate alias assignments for the entry module
    pub fn generate_alias_assignments(&self) -> Vec<Stmt> {
        let mut assignments = Vec::new();

        for (alias_name, import_alias) in &self.import_aliases {
            if import_alias.is_from_import {
                // Check if there's a conflict for this imported name
                let has_conflict = self
                    .name_conflicts
                    .contains_key(&import_alias.original_name);

                // Only generate assignment if:
                // 1. There was an explicit alias in the original code, OR
                // 2. There's a name conflict that requires renaming
                if import_alias.has_explicit_alias || has_conflict {
                    let actual_name = if let Some(conflict) =
                        self.name_conflicts.get(&import_alias.original_name)
                    {
                        // Use the renamed version for this module
                        conflict
                            .renamed_versions
                            .get(&import_alias.module_name)
                            .cloned()
                            .unwrap_or_else(|| import_alias.original_name.clone())
                    } else {
                        import_alias.original_name.clone()
                    };

                    let assignment = ast::StmtAssign {
                        targets: vec![Expr::Name(ast::ExprName {
                            id: alias_name.clone().into(),
                            ctx: ExprContext::Store,
                            range: Default::default(),
                        })],
                        value: Box::new(Expr::Name(ast::ExprName {
                            id: actual_name.into(),
                            ctx: ExprContext::Load,
                            range: Default::default(),
                        })),
                        type_comment: None,
                        range: Default::default(),
                    };
                    assignments.push(Stmt::Assign(assignment));
                }
            }
        }

        assignments
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
}

impl Default for AstRewriter {
    fn default() -> Self {
        Self::new()
    }
}
