use std::hash::BuildHasherDefault;

use indexmap::IndexMap;
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprAttribute, ExprContext, ExprName, Identifier, Stmt, StmtFunctionDef,
};
use ruff_text_size::TextRange;
use rustc_hash::{FxHashSet, FxHasher};

/// Type alias for IndexMap with FxHasher for better performance
type FxIndexMap<K, V> = IndexMap<K, V, BuildHasherDefault<FxHasher>>;

use crate::semantic_bundler::ModuleGlobalInfo;

/// Transformer that lifts module-level globals to true global scope
pub struct GlobalsLifter {
    /// Map from original name to lifted name
    pub lifted_names: FxIndexMap<String, String>,
    /// Statements to add at module top level
    pub lifted_declarations: Vec<Stmt>,
}

/// Transform globals() references in expressions
pub fn transform_globals_in_expr(expr: &mut Expr) {
    match expr {
        Expr::Call(call_expr) => {
            // Check if this is a globals() call
            if let Expr::Name(name_expr) = &*call_expr.func
                && name_expr.id.as_str() == "globals"
                && call_expr.arguments.args.is_empty()
            {
                // Replace the entire expression with module.__dict__
                *expr = Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "module".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("__dict__", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                });
                return;
            }

            // Recursively transform in function and arguments
            transform_globals_in_expr(&mut call_expr.func);
            for arg in &mut call_expr.arguments.args {
                transform_globals_in_expr(arg);
            }
            for keyword in &mut call_expr.arguments.keywords {
                transform_globals_in_expr(&mut keyword.value);
            }
        }
        Expr::Attribute(attr_expr) => {
            transform_globals_in_expr(&mut attr_expr.value);
        }
        Expr::Subscript(subscript_expr) => {
            transform_globals_in_expr(&mut subscript_expr.value);
            transform_globals_in_expr(&mut subscript_expr.slice);
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                transform_globals_in_expr(elem);
            }
        }
        Expr::Dict(dict_expr) => {
            for item in &mut dict_expr.items {
                if let Some(ref mut key) = item.key {
                    transform_globals_in_expr(key);
                }
                transform_globals_in_expr(&mut item.value);
            }
        }
        Expr::If(if_expr) => {
            transform_globals_in_expr(&mut if_expr.test);
            transform_globals_in_expr(&mut if_expr.body);
            transform_globals_in_expr(&mut if_expr.orelse);
        }
        // Add more expression types as needed
        _ => {}
    }
}

/// Transform globals() calls in a statement
pub fn transform_globals_in_stmt(stmt: &mut Stmt) {
    match stmt {
        Stmt::Expr(expr_stmt) => {
            transform_globals_in_expr(&mut expr_stmt.value);
        }
        Stmt::Assign(assign_stmt) => {
            transform_globals_in_expr(&mut assign_stmt.value);
            for target in &mut assign_stmt.targets {
                transform_globals_in_expr(target);
            }
        }
        Stmt::Return(return_stmt) => {
            if let Some(ref mut value) = return_stmt.value {
                transform_globals_in_expr(value);
            }
        }
        Stmt::If(if_stmt) => {
            transform_globals_in_expr(&mut if_stmt.test);
            for stmt in &mut if_stmt.body {
                transform_globals_in_stmt(stmt);
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(ref mut test_expr) = clause.test {
                    transform_globals_in_expr(test_expr);
                }
                for stmt in &mut clause.body {
                    transform_globals_in_stmt(stmt);
                }
            }
        }
        Stmt::FunctionDef(func_def) => {
            // Transform globals() calls in function body
            for stmt in &mut func_def.body {
                transform_globals_in_stmt(stmt);
            }
        }
        // Add more statement types as needed
        _ => {}
    }
}

impl Default for GlobalsLifter {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalsLifter {
    /// Create a new GlobalsLifter
    pub fn new() -> Self {
        Self {
            lifted_names: FxIndexMap::default(),
            lifted_declarations: Vec::new(),
        }
    }

    /// Create a new GlobalsLifter from module global info
    pub fn new_from_global_info(global_info: &ModuleGlobalInfo) -> Self {
        use cow_utils::CowUtils;
        let mut lifter = Self::new();
        // Process global declarations from the module
        for global_name in global_info.global_declarations.keys() {
            let lifted_name = format!(
                "__cribo_{}_{}",
                global_info.module_name.cow_replace('.', "_").into_owned(),
                global_name
            );
            lifter.lifted_names.insert(global_name.clone(), lifted_name);
        }
        lifter
    }

    /// Get the lifted names mapping
    pub fn get_lifted_names(&self) -> &FxIndexMap<String, String> {
        &self.lifted_names
    }

    /// Process a function and lift its globals
    pub fn process_function(
        &mut self,
        func: &mut StmtFunctionDef,
        module_name: &str,
        function_globals: &FxHashSet<String>,
    ) {
        // Create lifted names for this function's globals
        for global in function_globals {
            let lifted_name = format!("_bundler_global_{}_{}_{}", module_name, &func.name, global);
            self.lifted_names.insert(global.clone(), lifted_name);
        }

        // Transform the function body
        let mut transformer = FunctionGlobalTransformer {
            lifted_names: &self.lifted_names,
        };
        transformer.visit_body(&mut func.body);
    }

    /// Get the lifted declarations
    pub fn get_lifted_declarations(&self) -> &[Stmt] {
        &self.lifted_declarations
    }
}

/// Transformer that transforms global variable references to lifted names
struct FunctionGlobalTransformer<'a> {
    lifted_names: &'a FxIndexMap<String, String>,
}

impl<'a> FunctionGlobalTransformer<'a> {
    fn transform_expr(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Name(name) => {
                if let Some(lifted_name) = self.lifted_names.get(name.id.as_str()) {
                    name.id = Identifier::new(lifted_name.clone(), name.range).into();
                }
            }
            Expr::Attribute(attr) => {
                self.transform_expr(&mut attr.value);
            }
            Expr::Call(call) => {
                self.transform_expr(&mut call.func);
                for arg in &mut call.arguments.args {
                    self.transform_expr(arg);
                }
                for kw in &mut call.arguments.keywords {
                    self.transform_expr(&mut kw.value);
                }
            }
            Expr::BinOp(binop) => {
                self.transform_expr(&mut binop.left);
                self.transform_expr(&mut binop.right);
            }
            Expr::UnaryOp(unop) => {
                self.transform_expr(&mut unop.operand);
            }
            Expr::Subscript(sub) => {
                self.transform_expr(&mut sub.value);
                self.transform_expr(&mut sub.slice);
            }
            Expr::List(list) => {
                for elem in &mut list.elts {
                    self.transform_expr(elem);
                }
            }
            Expr::Tuple(tuple) => {
                for elem in &mut tuple.elts {
                    self.transform_expr(elem);
                }
            }
            Expr::Dict(dict) => {
                for item in &mut dict.items {
                    if let Some(key) = &mut item.key {
                        self.transform_expr(key);
                    }
                    self.transform_expr(&mut item.value);
                }
            }
            _ => {}
        }
    }

    fn transform_stmt(&mut self, stmt: &mut Stmt) {
        // Transform globals() calls in statements first
        transform_globals_in_stmt(stmt);

        // Then handle other transformations
        match stmt {
            Stmt::FunctionDef(func) => {
                for stmt in &mut func.body {
                    self.transform_stmt(stmt);
                }
            }
            Stmt::ClassDef(class) => {
                for stmt in &mut class.body {
                    self.transform_stmt(stmt);
                }
            }
            Stmt::If(if_stmt) => {
                self.transform_expr(&mut if_stmt.test);
                for stmt in &mut if_stmt.body {
                    self.transform_stmt(stmt);
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    for stmt in &mut clause.body {
                        self.transform_stmt(stmt);
                    }
                }
            }
            Stmt::While(while_stmt) => {
                self.transform_expr(&mut while_stmt.test);
                for stmt in &mut while_stmt.body {
                    self.transform_stmt(stmt);
                }
            }
            Stmt::For(for_stmt) => {
                self.transform_expr(&mut for_stmt.iter);
                for stmt in &mut for_stmt.body {
                    self.transform_stmt(stmt);
                }
            }
            Stmt::Expr(expr_stmt) => {
                self.transform_expr(&mut expr_stmt.value);
            }
            Stmt::Assign(assign) => {
                self.transform_expr(&mut assign.value);
                for target in &mut assign.targets {
                    self.transform_expr(target);
                }
            }
            Stmt::AnnAssign(ann) => {
                if let Some(value) = &mut ann.value {
                    self.transform_expr(value);
                }
            }
            Stmt::Return(ret) => {
                if let Some(value) = &mut ret.value {
                    self.transform_expr(value);
                }
            }
            _ => {}
        }
    }

    fn visit_body(&mut self, body: &mut Vec<Stmt>) {
        for stmt in body {
            self.transform_stmt(stmt);
        }
    }
}
