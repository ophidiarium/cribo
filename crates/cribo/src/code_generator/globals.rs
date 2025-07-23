use cow_utils::CowUtils;
use log::debug;
use ruff_python_ast::{Expr, ExprContext, Stmt};

use crate::{
    ast_builder::{expressions, statements},
    semantic_bundler::ModuleGlobalInfo,
    types::FxIndexMap,
};

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
                && call_expr.arguments.keywords.is_empty()
            {
                // Replace the entire expression with module.__dict__
                *expr = expressions::attribute(
                    expressions::name("module", ExprContext::Load),
                    "__dict__",
                    ExprContext::Load,
                );
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

impl GlobalsLifter {
    pub fn new(global_info: &ModuleGlobalInfo) -> Self {
        let mut lifted_names = FxIndexMap::default();
        let mut lifted_declarations = Vec::new();

        debug!("GlobalsLifter::new for module: {}", global_info.module_name);
        debug!("Module level vars: {:?}", global_info.module_level_vars);
        debug!(
            "Global declarations: {:?}",
            global_info.global_declarations.keys().collect::<Vec<_>>()
        );

        // Generate lifted names and declarations for all module-level variables
        // that are referenced with global statements
        for var_name in &global_info.module_level_vars {
            // Only lift variables that are actually used with global statements
            if global_info.global_declarations.contains_key(var_name) {
                let module_name_sanitized = global_info.module_name.cow_replace(".", "_");
                let module_name_sanitized = module_name_sanitized.cow_replace("-", "_");
                let lifted_name = format!("_cribo_{module_name_sanitized}_{var_name}");

                debug!("Creating lifted declaration for {var_name} -> {lifted_name}");

                lifted_names.insert(var_name.clone(), lifted_name.clone());

                // Create assignment: __cribo_module_var = None (will be set by init function)
                lifted_declarations.push(statements::simple_assign(
                    &lifted_name,
                    expressions::none_literal(),
                ));
            }
        }

        debug!("Created {} lifted declarations", lifted_declarations.len());

        Self {
            lifted_names,
            lifted_declarations,
        }
    }

    /// Get the lifted global declarations
    pub fn get_lifted_declarations(&self) -> Vec<Stmt> {
        self.lifted_declarations.clone()
    }

    /// Get the lifted names mapping
    pub fn get_lifted_names(&self) -> &FxIndexMap<String, String> {
        &self.lifted_names
    }
}
