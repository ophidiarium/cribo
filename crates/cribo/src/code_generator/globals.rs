use indexmap::IndexMap as FxIndexMap;
use ruff_python_ast::{Expr, ExprAttribute, ExprName, Identifier, Stmt, StmtFunctionDef};
use rustc_hash::FxHashSet;

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
        Expr::Call(call) => {
            // Check if this is globals()
            if let Expr::Name(name) = &call.func.as_ref() {
                if name.id.as_str() == "globals" && call.arguments.is_empty() {
                    // Replace globals() with _bundler_globals
                    *expr = Expr::Name(ExprName {
                        id: Identifier::new("_bundler_globals", name.range),
                        ctx: name.ctx.clone(),
                        range: call.range,
                        node_index: Default::default(),
                    });
                    return;
                }
            }
            // Process arguments
            for arg in &mut call.arguments.args {
                transform_globals_in_expr(arg);
            }
            for kw in &mut call.arguments.keywords {
                if let Some(arg) = &mut kw.value {
                    transform_globals_in_expr(arg);
                }
            }
            // Process the function itself
            transform_globals_in_expr(&mut call.func);
        }
        Expr::Attribute(attr) => {
            transform_globals_in_expr(&mut attr.value);
        }
        Expr::BinOp(binop) => {
            transform_globals_in_expr(&mut binop.left);
            transform_globals_in_expr(&mut binop.right);
        }
        Expr::UnaryOp(unop) => {
            transform_globals_in_expr(&mut unop.operand);
        }
        Expr::Lambda(lambda) => {
            transform_globals_in_expr(&mut lambda.body);
        }
        Expr::If(if_expr) => {
            transform_globals_in_expr(&mut if_expr.test);
            transform_globals_in_expr(&mut if_expr.body);
            transform_globals_in_expr(&mut if_expr.orelse);
        }
        Expr::Dict(dict) => {
            for item in &mut dict.items {
                if let Some(key) = &mut item.key {
                    transform_globals_in_expr(key);
                }
                transform_globals_in_expr(&mut item.value);
            }
        }
        Expr::List(list) => {
            for elem in &mut list.elts {
                transform_globals_in_expr(elem);
            }
        }
        Expr::Tuple(tuple) => {
            for elem in &mut tuple.elts {
                transform_globals_in_expr(elem);
            }
        }
        Expr::Subscript(sub) => {
            transform_globals_in_expr(&mut sub.value);
            transform_globals_in_expr(&mut sub.slice);
        }
        _ => {}
    }
}

/// Transform globals() references in statements
pub fn transform_globals_in_stmt(stmt: &mut Stmt) {
    match stmt {
        Stmt::Expr(stmt_expr) => {
            transform_globals_in_expr(&mut stmt_expr.value);
        }
        Stmt::Assign(assign) => {
            transform_globals_in_expr(&mut assign.value);
            for target in &mut assign.targets {
                transform_globals_in_expr(target);
            }
        }
        Stmt::AnnAssign(ann_assign) => {
            if let Some(value) = &mut ann_assign.value {
                transform_globals_in_expr(value);
            }
        }
        Stmt::Return(ret) => {
            if let Some(value) = &mut ret.value {
                transform_globals_in_expr(value);
            }
        }
        Stmt::If(if_stmt) => {
            transform_globals_in_expr(&mut if_stmt.test);
            for stmt in &mut if_stmt.body {
                transform_globals_in_stmt(stmt);
            }
            for clause in &mut if_stmt.elif_else_clauses {
                for stmt in &mut clause.body {
                    transform_globals_in_stmt(stmt);
                }
            }
        }
        Stmt::While(while_stmt) => {
            transform_globals_in_expr(&mut while_stmt.test);
            for stmt in &mut while_stmt.body {
                transform_globals_in_stmt(stmt);
            }
        }
        Stmt::For(for_stmt) => {
            transform_globals_in_expr(&mut for_stmt.iter);
            for stmt in &mut for_stmt.body {
                transform_globals_in_stmt(stmt);
            }
        }
        _ => {}
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
                    name.id = Identifier::new(lifted_name.clone(), name.range);
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
