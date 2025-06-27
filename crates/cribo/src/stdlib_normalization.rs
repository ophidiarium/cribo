use std::hash::BuildHasherDefault;

use indexmap::{IndexMap, IndexSet};
use log::debug;
use ruff_python_ast::{
    ExceptHandler, Expr, ExprAttribute, ExprContext, ExprName, Identifier, ModModule, Stmt,
    StmtAssign, StmtClassDef, StmtFunctionDef, StmtImport,
};
use ruff_text_size::TextRange;
use rustc_hash::FxHasher;

/// Type alias for IndexMap with FxHasher for better performance
type FxIndexMap<K, V> = IndexMap<K, V, BuildHasherDefault<FxHasher>>;
/// Type alias for IndexSet with FxHasher for better performance
type FxIndexSet<T> = IndexSet<T, BuildHasherDefault<FxHasher>>;

/// Result of stdlib normalization
pub struct NormalizationResult {
    /// Mapping of aliases to canonical names (e.g., "PyPath" -> "pathlib.Path")
    pub alias_to_canonical: FxIndexMap<String, String>,
    /// Set of modules that were created by normalization (e.g., "abc", "collections")
    pub normalized_modules: FxIndexSet<String>,
}

/// Normalizes stdlib import aliases within a module's AST
/// Converts "import json as j" to "import json" and rewrites all "j.dumps" to "json.dumps"
/// Also converts "from pathlib import Path as PyPath" to "import pathlib" and rewrites "PyPath" to
/// "pathlib.Path"
pub fn normalize_stdlib_imports(ast: &mut ModModule) -> NormalizationResult {
    let normalizer = StdlibNormalizer::new();
    normalizer.normalize(ast)
}

struct StdlibNormalizer {
    // No state needed for now
}

impl StdlibNormalizer {
    fn new() -> Self {
        Self {}
    }

    /// Main normalization entry point
    fn normalize(&self, ast: &mut ModModule) -> NormalizationResult {
        // Step 1: Build alias-to-canonical mapping for this file
        let mut alias_to_canonical = FxIndexMap::default();
        let mut modules_to_convert = FxIndexSet::default();

        for stmt in &ast.body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    self.collect_stdlib_aliases(import_stmt, &mut alias_to_canonical);
                }
                Stmt::ImportFrom(import_from) => {
                    // Skip relative imports
                    if import_from.level > 0 {
                        continue;
                    }

                    if let Some(ref module) = import_from.module {
                        let module_name = module.as_str();
                        if self.is_safe_stdlib_module(module_name) {
                            // Extract the root module for stdlib imports
                            let root_module = module_name.split('.').next().unwrap_or(module_name);

                            // Collect all imports from "from" statements for normalization
                            for alias in &import_from.names {
                                let name = alias.name.as_str();
                                if let Some(ref alias_name) = alias.asname {
                                    // Map alias to module.name (e.g., PyPath -> pathlib.Path)
                                    let canonical = format!("{module_name}.{name}");
                                    alias_to_canonical
                                        .insert(alias_name.as_str().to_string(), canonical);
                                } else {
                                    // Even without alias, we need to convert to module.name form
                                    // e.g., "Any" -> "typing.Any"
                                    let canonical = format!("{module_name}.{name}");
                                    alias_to_canonical.insert(name.to_string(), canonical);
                                }
                            }

                            // Convert ALL stdlib "from" imports to regular imports
                            // This applies to typing, collections.abc, etc.
                            modules_to_convert.insert(root_module.to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        if alias_to_canonical.is_empty() && modules_to_convert.is_empty() {
            return NormalizationResult {
                alias_to_canonical,
                normalized_modules: FxIndexSet::default(),
            };
        }

        debug!("Normalizing stdlib aliases: {alias_to_canonical:?}");
        debug!("Modules to convert from 'from' imports: {modules_to_convert:?}");

        // Step 2: Transform all expressions that reference aliases
        for (idx, stmt) in ast.body.iter_mut().enumerate() {
            match stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    // We'll handle import statements separately
                }
                _ => {
                    let stmt_type = match stmt {
                        Stmt::FunctionDef(f) => format!("FunctionDef({})", f.name.as_str()),
                        Stmt::ClassDef(c) => format!("ClassDef({})", c.name.as_str()),
                        Stmt::Assign(_) => "Assign".to_string(),
                        Stmt::Expr(_) => "Expr".to_string(),
                        _ => format!("{:?}", std::mem::discriminant(stmt)),
                    };
                    debug!("Rewriting aliases in statement at index {idx}: {stmt_type}");
                    self.rewrite_aliases_in_stmt(stmt, &alias_to_canonical);
                }
            }
        }

        // Step 3: Transform import statements
        let mut new_imports = Vec::new();
        let mut indices_to_remove = Vec::new();
        // Track assignments we need to add for implicit exports
        // Maps module name to list of (local_name, full_path) tuples
        let mut implicit_exports: FxIndexMap<String, Vec<(String, String)>> = FxIndexMap::default();

        for (idx, stmt) in ast.body.iter_mut().enumerate() {
            match stmt {
                Stmt::Import(import_stmt) => {
                    debug!(
                        "Processing import statement at index {}: {:?}",
                        idx,
                        import_stmt
                            .names
                            .iter()
                            .map(|a| (a.name.as_str(), a.asname.as_ref().map(|n| n.as_str())))
                            .collect::<Vec<_>>()
                    );
                    self.normalize_import_aliases(import_stmt);
                }
                Stmt::ImportFrom(import_from) => {
                    // Skip relative imports
                    if import_from.level > 0 {
                        continue;
                    }

                    if let Some(ref module) = import_from.module {
                        let module_name = module.as_str();
                        // Check if this is a safe stdlib module or submodule
                        if self.is_safe_stdlib_module(module_name) {
                            // Extract the root module name
                            let root_module = module_name.split('.').next().unwrap_or(module_name);

                            if modules_to_convert.contains(root_module) {
                                // Mark this import for removal - we'll convert it to a regular
                                // import
                                indices_to_remove.push(idx);

                                // For submodules like collections.abc, we need to import the full
                                // module path not just the root
                                // module
                                if !new_imports.iter().any(|m: &String| m == module_name) {
                                    new_imports.push(module_name.to_string());
                                }

                                // Collect implicit exports that need assignment statements
                                // e.g., from collections.abc import MutableMapping
                                // becomes: import collections.abc + MutableMapping =
                                // collections.abc.MutableMapping
                                self.process_import_from_names(
                                    import_from,
                                    module_name,
                                    &mut new_imports,
                                    &mut implicit_exports,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Step 4: Remove the from imports and add regular imports
        // Remove in reverse order to maintain indices
        for idx in indices_to_remove.into_iter().rev() {
            ast.body.remove(idx);
        }

        // Add the new regular imports at the beginning (after __future__ imports)
        let future_import_count = ast
            .body
            .iter()
            .take_while(|stmt| {
                if let Stmt::ImportFrom(import_from) = stmt {
                    import_from
                        .module
                        .as_ref()
                        .is_some_and(|m| m.as_str() == "__future__")
                } else {
                    false
                }
            })
            .count();

        let mut insert_position = future_import_count;
        let mut normalized_modules = FxIndexSet::default();

        for module_name in new_imports.into_iter().rev() {
            // Track this module as normalized
            normalized_modules.insert(module_name.clone());

            let import_stmt = Stmt::Import(StmtImport {
                node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                names: vec![ruff_python_ast::Alias {
                    node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                    name: Identifier::new(&module_name, TextRange::default()),
                    asname: None,
                    range: TextRange::default(),
                }],
                range: TextRange::default(),
            });
            ast.body.insert(insert_position, import_stmt);
            insert_position += 1;

            // Add assignment statements for implicit exports from this module
            if let Some(exports) = implicit_exports.get(&module_name) {
                for (local_name, full_path) in exports {
                    let assign_stmt = self.create_assignment_statement(local_name, full_path);
                    ast.body.insert(insert_position, assign_stmt);
                    insert_position += 1;
                }
            }
        }

        NormalizationResult {
            alias_to_canonical,
            normalized_modules,
        }
    }

    /// Check if a module is safe to hoist
    fn is_safe_stdlib_module(&self, module_name: &str) -> bool {
        // For now, only consider __future__ as safe to hoist
        // The proper way would be to check the module metadata from the graph
        // but this module doesn't have access to the graph
        module_name == "__future__"
    }

    /// Check if a path refers to a known stdlib submodule
    fn is_known_stdlib_submodule(&self, module_path: &str) -> bool {
        // Check if this is a stdlib module itself (not just an attribute)
        match module_path {
            // Known stdlib submodules that need separate imports
            "http.cookiejar" | "http.cookies" | "http.server" | "http.client" => true,
            "urllib.parse" | "urllib.request" | "urllib.response" | "urllib.error"
            | "urllib.robotparser" => true,
            "xml.etree" | "xml.etree.ElementTree" | "xml.dom" | "xml.sax" | "xml.parsers" => true,
            "email.mime" | "email.parser" | "email.message" | "email.utils" => true,
            "collections.abc" => true,
            "concurrent.futures" => true,
            "importlib.util" | "importlib.machinery" | "importlib.resources" => true,
            "multiprocessing.pool" | "multiprocessing.managers" => true,
            "os.path" => true,
            _ => {
                // For other cases, check if it's a known stdlib module
                let root = module_path.split('.').next().unwrap_or(module_path);
                if ruff_python_stdlib::sys::is_known_standard_library(10, root) {
                    // If the root is stdlib and the full path is also recognized as stdlib,
                    // it's likely a submodule
                    ruff_python_stdlib::sys::is_known_standard_library(10, module_path)
                } else {
                    false
                }
            }
        }
    }

    /// Create an assignment statement: local_name = full_path
    fn create_assignment_statement(&self, local_name: &str, full_path: &str) -> Stmt {
        // Parse the full path to create attribute access
        // e.g., "collections.abc.MutableMapping" becomes collections.abc.MutableMapping
        let parts: Vec<&str> = full_path.split('.').collect();

        let mut value_expr = Expr::Name(ExprName {
            node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
            id: parts[0].into(),
            ctx: ExprContext::Load,
            range: TextRange::default(),
        });

        // Build nested attribute access for remaining parts
        for part in &parts[1..] {
            value_expr = Expr::Attribute(ExprAttribute {
                node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                value: Box::new(value_expr),
                attr: Identifier::new(*part, TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            });
        }

        // Create the target (local_name)
        let target = Expr::Name(ExprName {
            node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
            id: local_name.into(),
            ctx: ExprContext::Store,
            range: TextRange::default(),
        });

        Stmt::Assign(StmtAssign {
            node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
            targets: vec![target],
            value: Box::new(value_expr),
            range: TextRange::default(),
        })
    }

    /// Collect stdlib aliases from import statement
    fn collect_stdlib_aliases(
        &self,
        import_stmt: &StmtImport,
        alias_to_canonical: &mut FxIndexMap<String, String>,
    ) {
        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();
            if !self.is_safe_stdlib_module(module_name) {
                continue;
            }
            if let Some(ref alias_name) = alias.asname {
                // This is an aliased import: import json as j
                alias_to_canonical.insert(alias_name.as_str().to_string(), module_name.to_string());
            }
        }
    }

    /// Normalize import aliases by removing them for stdlib modules
    fn normalize_import_aliases(&self, import_stmt: &mut StmtImport) {
        for alias in &mut import_stmt.names {
            let module_name = alias.name.as_str();
            if !self.is_safe_stdlib_module(module_name) {
                debug!("Skipping non-safe stdlib module: {module_name}");
                continue;
            }
            if alias.asname.is_none() {
                continue;
            }
            // Remove the alias, keeping only the canonical name
            alias.asname = None;
            debug!("Normalized import to canonical: import {module_name}");
        }
    }

    /// Recursively rewrite aliases in statements
    fn rewrite_aliases_in_stmt(
        &self,
        stmt: &mut Stmt,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::Expr(expr_stmt) => {
                Self::rewrite_aliases_in_expr(&mut expr_stmt.value, alias_to_canonical);
            }
            Stmt::Assign(assign) => {
                Self::rewrite_aliases_in_expr(&mut assign.value, alias_to_canonical);
                for target in &mut assign.targets {
                    Self::rewrite_aliases_in_expr(target, alias_to_canonical);
                }
            }
            Stmt::Return(return_stmt) => {
                debug!("Rewriting aliases in return statement");
                if let Some(ref mut value) = return_stmt.value {
                    Self::rewrite_aliases_in_expr(value, alias_to_canonical);
                }
            }
            Stmt::FunctionDef(func_def) => {
                self.rewrite_aliases_in_function(func_def, alias_to_canonical);
            }
            Stmt::ClassDef(class_def) => {
                self.rewrite_aliases_in_class(class_def, alias_to_canonical);
            }
            Stmt::AnnAssign(ann_assign) => {
                // Rewrite the annotation
                Self::rewrite_aliases_in_expr(&mut ann_assign.annotation, alias_to_canonical);
                // Rewrite the target
                Self::rewrite_aliases_in_expr(&mut ann_assign.target, alias_to_canonical);
                // Rewrite the value if present
                if let Some(ref mut value) = ann_assign.value {
                    Self::rewrite_aliases_in_expr(value, alias_to_canonical);
                }
            }
            Stmt::If(if_stmt) => {
                Self::rewrite_aliases_in_expr(&mut if_stmt.test, alias_to_canonical);
                for stmt in &mut if_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(ref mut condition) = clause.test {
                        Self::rewrite_aliases_in_expr(condition, alias_to_canonical);
                    }
                    for stmt in &mut clause.body {
                        self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                    }
                }
            }
            Stmt::While(while_stmt) => {
                Self::rewrite_aliases_in_expr(&mut while_stmt.test, alias_to_canonical);
                for stmt in &mut while_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for stmt in &mut while_stmt.orelse {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::For(for_stmt) => {
                Self::rewrite_aliases_in_expr(&mut for_stmt.iter, alias_to_canonical);
                for stmt in &mut for_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for stmt in &mut for_stmt.orelse {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::With(with_stmt) => {
                for item in &mut with_stmt.items {
                    Self::rewrite_aliases_in_expr(&mut item.context_expr, alias_to_canonical);
                }
                for stmt in &mut with_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::Try(try_stmt) => {
                for stmt in &mut try_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for handler in &mut try_stmt.handlers {
                    self.rewrite_aliases_in_except_handler(handler, alias_to_canonical);
                }
                for stmt in &mut try_stmt.orelse {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for stmt in &mut try_stmt.finalbody {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::AugAssign(aug_assign) => {
                Self::rewrite_aliases_in_expr(&mut aug_assign.target, alias_to_canonical);
                Self::rewrite_aliases_in_expr(&mut aug_assign.value, alias_to_canonical);
            }
            Stmt::Raise(raise_stmt) => {
                if let Some(ref mut exc) = raise_stmt.exc {
                    Self::rewrite_aliases_in_expr(exc, alias_to_canonical);
                }
                if let Some(ref mut cause) = raise_stmt.cause {
                    Self::rewrite_aliases_in_expr(cause, alias_to_canonical);
                }
            }
            Stmt::Assert(assert_stmt) => {
                Self::rewrite_aliases_in_expr(&mut assert_stmt.test, alias_to_canonical);
                if let Some(ref mut msg) = assert_stmt.msg {
                    Self::rewrite_aliases_in_expr(msg, alias_to_canonical);
                }
            }
            Stmt::Delete(delete_stmt) => {
                for target in &mut delete_stmt.targets {
                    Self::rewrite_aliases_in_expr(target, alias_to_canonical);
                }
            }
            Stmt::Global(_)
            | Stmt::Nonlocal(_)
            | Stmt::Pass(_)
            | Stmt::Break(_)
            | Stmt::Continue(_) => {
                // These statements don't contain expressions to rewrite
            }
            // Handle other statement types as needed
            _ => {
                debug!(
                    "Unhandled statement type in rewrite_aliases_in_stmt: {:?}",
                    std::mem::discriminant(stmt)
                );
            }
        }
    }

    /// Rewrite aliases in expressions
    fn rewrite_aliases_in_expr(expr: &mut Expr, alias_to_canonical: &FxIndexMap<String, String>) {
        match expr {
            Expr::Name(name_expr) if matches!(name_expr.ctx, ExprContext::Load) => {
                // Check if this is an aliased import that should be rewritten
                if let Some(canonical) = alias_to_canonical.get(name_expr.id.as_str()) {
                    debug!(
                        "Rewriting name '{}' to '{}'",
                        name_expr.id.as_str(),
                        canonical
                    );
                    // Replace simple name with the canonical form
                    if canonical.contains('.') {
                        // Convert to attribute access (e.g., pathlib.Path)
                        let parts: Vec<&str> = canonical.split('.').collect();
                        let mut result = Expr::Name(ExprName {
                            node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                            id: parts[0].into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        });
                        for part in &parts[1..] {
                            result = Expr::Attribute(ExprAttribute {
                                node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                                value: Box::new(result),
                                attr: Identifier::new(*part, TextRange::default()),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            });
                        }
                        *expr = result;
                    } else {
                        // Simple module name
                        name_expr.id = canonical.clone().into();
                    }
                }
            }
            Expr::Attribute(attr_expr) => {
                // Check if the base is an aliased module
                if let Expr::Name(name_expr) = &mut *attr_expr.value {
                    if let Some(canonical) = alias_to_canonical.get(name_expr.id.as_str()) {
                        debug!(
                            "Rewriting attribute base '{}' to '{}' in attribute expression",
                            name_expr.id.as_str(),
                            canonical
                        );
                        name_expr.id = canonical.clone().into();
                    }
                } else {
                    debug!(
                        "Attribute base is not a Name expression, it's {:?}",
                        std::mem::discriminant(&*attr_expr.value)
                    );
                }
                // Recursively process the value
                Self::rewrite_aliases_in_expr(&mut attr_expr.value, alias_to_canonical);
            }
            Expr::Call(call_expr) => {
                // Debug the function being called
                if let Expr::Name(name) = &*call_expr.func {
                    debug!("Call expression with function name: {}", name.id.as_str());
                } else if let Expr::Attribute(attr) = &*call_expr.func
                    && let Expr::Name(base) = &*attr.value
                {
                    debug!(
                        "Call expression with attribute: {}.{}",
                        base.id.as_str(),
                        attr.attr.as_str()
                    );
                }

                Self::rewrite_aliases_in_expr(&mut call_expr.func, alias_to_canonical);
                for (i, arg) in call_expr.arguments.args.iter_mut().enumerate() {
                    debug!("  Rewriting call arg {i}");
                    Self::rewrite_aliases_in_expr(arg, alias_to_canonical);
                }
                for keyword in &mut call_expr.arguments.keywords {
                    Self::rewrite_aliases_in_expr(&mut keyword.value, alias_to_canonical);
                }
            }
            // Handle other expression types recursively
            Expr::List(list_expr) => {
                debug!("Rewriting aliases in list expression");
                for elem in &mut list_expr.elts {
                    Self::rewrite_aliases_in_expr(elem, alias_to_canonical);
                }
            }
            Expr::Tuple(tuple_expr) => {
                debug!("Rewriting aliases in tuple expression");
                for elem in &mut tuple_expr.elts {
                    Self::rewrite_aliases_in_expr(elem, alias_to_canonical);
                }
            }
            Expr::Subscript(subscript_expr) => {
                debug!("Rewriting aliases in subscript expression");
                Self::rewrite_aliases_in_expr(&mut subscript_expr.value, alias_to_canonical);
                Self::rewrite_aliases_in_expr(&mut subscript_expr.slice, alias_to_canonical);
            }
            Expr::BinOp(binop) => {
                Self::rewrite_aliases_in_expr(&mut binop.left, alias_to_canonical);
                Self::rewrite_aliases_in_expr(&mut binop.right, alias_to_canonical);
            }
            Expr::UnaryOp(unaryop) => {
                Self::rewrite_aliases_in_expr(&mut unaryop.operand, alias_to_canonical);
            }
            Expr::Compare(compare) => {
                Self::rewrite_aliases_in_expr(&mut compare.left, alias_to_canonical);
                for comparator in &mut compare.comparators {
                    Self::rewrite_aliases_in_expr(comparator, alias_to_canonical);
                }
            }
            Expr::BoolOp(boolop) => {
                for value in &mut boolop.values {
                    Self::rewrite_aliases_in_expr(value, alias_to_canonical);
                }
            }
            Expr::Dict(dict) => {
                for item in &mut dict.items {
                    if let Some(ref mut key) = item.key {
                        Self::rewrite_aliases_in_expr(key, alias_to_canonical);
                    }
                    Self::rewrite_aliases_in_expr(&mut item.value, alias_to_canonical);
                }
            }
            Expr::Set(set) => {
                for elem in &mut set.elts {
                    Self::rewrite_aliases_in_expr(elem, alias_to_canonical);
                }
            }
            Expr::ListComp(comp) => {
                Self::rewrite_aliases_in_expr(&mut comp.elt, alias_to_canonical);
                for generator in &mut comp.generators {
                    Self::rewrite_aliases_in_expr(&mut generator.iter, alias_to_canonical);
                    for if_clause in &mut generator.ifs {
                        Self::rewrite_aliases_in_expr(if_clause, alias_to_canonical);
                    }
                }
            }
            Expr::SetComp(comp) => {
                Self::rewrite_aliases_in_expr(&mut comp.elt, alias_to_canonical);
                for generator in &mut comp.generators {
                    Self::rewrite_aliases_in_expr(&mut generator.iter, alias_to_canonical);
                    for if_clause in &mut generator.ifs {
                        Self::rewrite_aliases_in_expr(if_clause, alias_to_canonical);
                    }
                }
            }
            Expr::DictComp(comp) => {
                Self::rewrite_aliases_in_expr(&mut comp.key, alias_to_canonical);
                Self::rewrite_aliases_in_expr(&mut comp.value, alias_to_canonical);
                for generator in &mut comp.generators {
                    Self::rewrite_aliases_in_expr(&mut generator.iter, alias_to_canonical);
                    for if_clause in &mut generator.ifs {
                        Self::rewrite_aliases_in_expr(if_clause, alias_to_canonical);
                    }
                }
            }
            Expr::Generator(comp) => {
                Self::rewrite_aliases_in_expr(&mut comp.elt, alias_to_canonical);
                for generator in &mut comp.generators {
                    Self::rewrite_aliases_in_expr(&mut generator.iter, alias_to_canonical);
                    for if_clause in &mut generator.ifs {
                        Self::rewrite_aliases_in_expr(if_clause, alias_to_canonical);
                    }
                }
            }
            Expr::Lambda(lambda) => {
                // Lambda parameters might have annotations
                if let Some(ref mut params) = lambda.parameters {
                    for param in &mut params.posonlyargs {
                        if let Some(ref mut annotation) = param.parameter.annotation {
                            Self::rewrite_aliases_in_expr(annotation, alias_to_canonical);
                        }
                    }
                    for param in &mut params.args {
                        if let Some(ref mut annotation) = param.parameter.annotation {
                            Self::rewrite_aliases_in_expr(annotation, alias_to_canonical);
                        }
                    }
                    for param in &mut params.kwonlyargs {
                        if let Some(ref mut annotation) = param.parameter.annotation {
                            Self::rewrite_aliases_in_expr(annotation, alias_to_canonical);
                        }
                    }
                }
                // Process the body
                Self::rewrite_aliases_in_expr(&mut lambda.body, alias_to_canonical);
            }
            Expr::If(ifexp) => {
                Self::rewrite_aliases_in_expr(&mut ifexp.test, alias_to_canonical);
                Self::rewrite_aliases_in_expr(&mut ifexp.body, alias_to_canonical);
                Self::rewrite_aliases_in_expr(&mut ifexp.orelse, alias_to_canonical);
            }
            Expr::Yield(yield_expr) => {
                if let Some(ref mut value) = yield_expr.value {
                    Self::rewrite_aliases_in_expr(value, alias_to_canonical);
                }
            }
            Expr::YieldFrom(yield_from) => {
                Self::rewrite_aliases_in_expr(&mut yield_from.value, alias_to_canonical);
            }
            Expr::Await(await_expr) => {
                Self::rewrite_aliases_in_expr(&mut await_expr.value, alias_to_canonical);
            }
            Expr::Starred(starred) => {
                Self::rewrite_aliases_in_expr(&mut starred.value, alias_to_canonical);
            }
            Expr::Slice(slice) => {
                if let Some(ref mut lower) = slice.lower {
                    Self::rewrite_aliases_in_expr(lower, alias_to_canonical);
                }
                if let Some(ref mut upper) = slice.upper {
                    Self::rewrite_aliases_in_expr(upper, alias_to_canonical);
                }
                if let Some(ref mut step) = slice.step {
                    Self::rewrite_aliases_in_expr(step, alias_to_canonical);
                }
            }
            Expr::Named(named) => {
                Self::rewrite_aliases_in_expr(&mut named.value, alias_to_canonical);
            }
            // Literals don't need rewriting
            Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_) => {
                // No aliases to rewrite in literals
            }
            _ => {
                debug!(
                    "Unhandled expression type in rewrite_aliases_in_expr: {:?}",
                    std::mem::discriminant(expr)
                );
            }
        }
    }

    /// Rewrite aliases in exception handlers
    fn rewrite_aliases_in_except_handler(
        &self,
        handler: &mut ExceptHandler,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        match handler {
            ExceptHandler::ExceptHandler(except_handler) => {
                if let Some(ref mut type_) = except_handler.type_ {
                    Self::rewrite_aliases_in_expr(type_, alias_to_canonical);
                }
                for stmt in &mut except_handler.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
        }
    }

    /// Rewrite aliases in function definitions
    fn rewrite_aliases_in_function(
        &self,
        func_def: &mut StmtFunctionDef,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        debug!("Rewriting aliases in function: {}", func_def.name.as_str());

        // Rewrite parameter annotations
        for param in &mut func_def.parameters.posonlyargs {
            if let Some(ref mut annotation) = param.parameter.annotation {
                Self::rewrite_aliases_in_expr(annotation, alias_to_canonical);
            }
        }
        for param in &mut func_def.parameters.args {
            if let Some(ref mut annotation) = param.parameter.annotation {
                Self::rewrite_aliases_in_expr(annotation, alias_to_canonical);
            }
        }
        for param in &mut func_def.parameters.kwonlyargs {
            if let Some(ref mut annotation) = param.parameter.annotation {
                Self::rewrite_aliases_in_expr(annotation, alias_to_canonical);
            }
        }
        if let Some(ref mut vararg) = func_def.parameters.vararg
            && let Some(ref mut annotation) = vararg.annotation
        {
            Self::rewrite_aliases_in_expr(annotation, alias_to_canonical);
        }
        if let Some(ref mut kwarg) = func_def.parameters.kwarg
            && let Some(ref mut annotation) = kwarg.annotation
        {
            Self::rewrite_aliases_in_expr(annotation, alias_to_canonical);
        }

        // Rewrite return type annotation
        if let Some(ref mut returns) = func_def.returns {
            Self::rewrite_aliases_in_expr(returns, alias_to_canonical);
        }

        // First handle global statements specially
        self.rewrite_global_statements_in_function(func_def, alias_to_canonical);

        // Then rewrite the rest of the function body
        for (idx, stmt) in func_def.body.iter_mut().enumerate() {
            match stmt {
                Stmt::Global(_) => {
                    // Already handled above
                }
                _ => {
                    debug!(
                        "  Rewriting aliases in function body statement {}: {:?}",
                        idx,
                        std::mem::discriminant(stmt)
                    );
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
        }
    }

    /// Rewrite only global statements in function, not other references
    fn rewrite_global_statements_in_function(
        &self,
        func_def: &mut StmtFunctionDef,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        for stmt in &mut func_def.body {
            Self::rewrite_global_statements_only(stmt, alias_to_canonical);
        }
    }

    /// Recursively rewrite only global statements, not other name references
    fn rewrite_global_statements_only(
        stmt: &mut Stmt,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::Global(global_stmt) => {
                // Apply renames to global variable names
                for name in &mut global_stmt.names {
                    let name_str = name.as_str();
                    if let Some(new_name) = alias_to_canonical.get(name_str) {
                        debug!("Rewriting global statement variable '{name_str}' to '{new_name}'");
                        *name = Identifier::new(new_name, TextRange::default());
                    }
                }
            }
            // For control flow statements, recurse into their bodies
            Stmt::If(if_stmt) => {
                for stmt in &mut if_stmt.body {
                    Self::rewrite_global_statements_only(stmt, alias_to_canonical);
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    for stmt in &mut clause.body {
                        Self::rewrite_global_statements_only(stmt, alias_to_canonical);
                    }
                }
            }
            // Handle other control flow statements similarly...
            _ => {}
        }
    }

    /// Rewrite aliases in class definitions
    fn rewrite_aliases_in_class(
        &self,
        class_def: &mut StmtClassDef,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        // Rewrite base classes
        if let Some(arguments) = &mut class_def.arguments {
            for base in &mut arguments.args {
                Self::rewrite_aliases_in_expr(base, alias_to_canonical);
            }
        }

        // Rewrite class body
        for stmt in &mut class_def.body {
            self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
        }
    }

    /// Process names from an import-from statement and collect imports and exports
    fn process_import_from_names(
        &self,
        import_from: &ruff_python_ast::StmtImportFrom,
        module_name: &str,
        new_imports: &mut Vec<String>,
        implicit_exports: &mut FxIndexMap<String, Vec<(String, String)>>,
    ) {
        for alias in &import_from.names {
            let name = alias.name.as_str();
            if name == "*" {
                continue; // Skip star imports
            }

            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

            // Check if this is importing a submodule (e.g., from http import cookiejar)
            let submodule_path = format!("{module_name}.{name}");
            if self.is_known_stdlib_submodule(&submodule_path) {
                self.handle_submodule_import(
                    &submodule_path,
                    local_name,
                    module_name,
                    new_imports,
                    implicit_exports,
                );
            } else {
                // Regular attribute import
                let full_path = format!("{module_name}.{name}");
                implicit_exports
                    .entry(module_name.to_string())
                    .or_default()
                    .push((local_name.to_string(), full_path));
            }
        }
    }

    /// Handle submodule imports by adding to new_imports and implicit_exports
    fn handle_submodule_import(
        &self,
        submodule_path: &str,
        local_name: &str,
        module_name: &str,
        new_imports: &mut Vec<String>,
        implicit_exports: &mut FxIndexMap<String, Vec<(String, String)>>,
    ) {
        // This is a submodule import, we need to import it separately
        if !new_imports.iter().any(|m: &String| m == submodule_path) {
            new_imports.push(submodule_path.to_string());
        }
        // And create assignment: local_name = submodule_path
        implicit_exports
            .entry(module_name.to_string())
            .or_default()
            .push((local_name.to_string(), submodule_path.to_string()));
    }
}
