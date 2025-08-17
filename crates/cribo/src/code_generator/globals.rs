use log::debug;
use ruff_python_ast::{ExceptHandler, Expr, ExprContext, Stmt};

use crate::{
    ast_builder::{expressions, statements},
    code_generator::{
        context::ProcessGlobalsParams, module_registry::sanitize_module_name_for_identifier,
    },
    semantic_bundler::ModuleGlobalInfo,
    types::FxIndexMap,
};

/// Sanitize a variable name for use in a Python identifier
/// This ensures variable names only contain valid Python identifier characters
fn sanitize_var_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            // Replace common invalid characters with underscore
            c if c.is_alphanumeric() || c == '_' => c,
            _ => '_',
        })
        .collect()
}

/// Transformer that lifts module-level globals to true global scope
pub struct GlobalsLifter {
    /// Map from original name to lifted name
    pub lifted_names: FxIndexMap<String, String>,
    /// Statements to add at module top level
    pub lifted_declarations: Vec<Stmt>,
}

/// Unified function to transform module-level introspection calls
/// For `locals()`: transforms to `vars(__cribo_module)`, stops at function/class boundaries
/// For `globals()`: transforms to `__cribo_module.__dict__`, recurses into all contexts
fn transform_introspection_in_expr(expr: &mut Expr, target_fn: &str, recurse_into_scopes: bool) {
    match expr {
        Expr::Call(call_expr) => {
            // Check if this is the target introspection call
            if let Expr::Name(name_expr) = &*call_expr.func
                && name_expr.id.as_str() == target_fn
                && call_expr.arguments.args.is_empty()
                && call_expr.arguments.keywords.is_empty()
            {
                // Transform based on the target function
                if target_fn == "locals" {
                    // Replace with vars(__cribo_module)
                    *expr = expressions::call(
                        expressions::name("vars", ExprContext::Load),
                        vec![expressions::name(
                            crate::code_generator::module_registry::MODULE_VAR,
                            ExprContext::Load,
                        )],
                        vec![],
                    );
                } else if target_fn == "globals" {
                    // Replace with __cribo_module.__dict__
                    *expr = expressions::attribute(
                        expressions::name(
                            crate::code_generator::module_registry::MODULE_VAR,
                            ExprContext::Load,
                        ),
                        "__dict__",
                        ExprContext::Load,
                    );
                }
                return;
            }

            // Recursively transform in function and arguments
            transform_introspection_in_expr(&mut call_expr.func, target_fn, recurse_into_scopes);
            for arg in &mut call_expr.arguments.args {
                transform_introspection_in_expr(arg, target_fn, recurse_into_scopes);
            }
            for keyword in &mut call_expr.arguments.keywords {
                transform_introspection_in_expr(&mut keyword.value, target_fn, recurse_into_scopes);
            }
        }
        Expr::Lambda(lambda_expr) if recurse_into_scopes => {
            // Only recurse into lambda if allowed (for globals)
            transform_introspection_in_expr(&mut lambda_expr.body, target_fn, recurse_into_scopes);
        }
        Expr::Attribute(attr_expr) => {
            transform_introspection_in_expr(&mut attr_expr.value, target_fn, recurse_into_scopes);
        }
        Expr::Subscript(subscript_expr) => {
            transform_introspection_in_expr(
                &mut subscript_expr.value,
                target_fn,
                recurse_into_scopes,
            );
            transform_introspection_in_expr(
                &mut subscript_expr.slice,
                target_fn,
                recurse_into_scopes,
            );
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                transform_introspection_in_expr(elem, target_fn, recurse_into_scopes);
            }
        }
        Expr::Dict(dict_expr) => {
            for item in &mut dict_expr.items {
                if let Some(ref mut key) = item.key {
                    transform_introspection_in_expr(key, target_fn, recurse_into_scopes);
                }
                transform_introspection_in_expr(&mut item.value, target_fn, recurse_into_scopes);
            }
        }
        Expr::If(if_expr) => {
            transform_introspection_in_expr(&mut if_expr.test, target_fn, recurse_into_scopes);
            transform_introspection_in_expr(&mut if_expr.body, target_fn, recurse_into_scopes);
            transform_introspection_in_expr(&mut if_expr.orelse, target_fn, recurse_into_scopes);
        }
        Expr::ListComp(comp_expr) => {
            transform_introspection_in_expr(&mut comp_expr.elt, target_fn, recurse_into_scopes);
            for generator in &mut comp_expr.generators {
                transform_introspection_in_expr(
                    &mut generator.iter,
                    target_fn,
                    recurse_into_scopes,
                );
                transform_introspection_in_expr(
                    &mut generator.target,
                    target_fn,
                    recurse_into_scopes,
                );
                for if_clause in &mut generator.ifs {
                    transform_introspection_in_expr(if_clause, target_fn, recurse_into_scopes);
                }
            }
        }
        Expr::DictComp(comp_expr) => {
            transform_introspection_in_expr(&mut comp_expr.key, target_fn, recurse_into_scopes);
            transform_introspection_in_expr(&mut comp_expr.value, target_fn, recurse_into_scopes);
            for generator in &mut comp_expr.generators {
                transform_introspection_in_expr(
                    &mut generator.iter,
                    target_fn,
                    recurse_into_scopes,
                );
                transform_introspection_in_expr(
                    &mut generator.target,
                    target_fn,
                    recurse_into_scopes,
                );
                for if_clause in &mut generator.ifs {
                    transform_introspection_in_expr(if_clause, target_fn, recurse_into_scopes);
                }
            }
        }
        Expr::SetComp(comp_expr) => {
            transform_introspection_in_expr(&mut comp_expr.elt, target_fn, recurse_into_scopes);
            for generator in &mut comp_expr.generators {
                transform_introspection_in_expr(
                    &mut generator.iter,
                    target_fn,
                    recurse_into_scopes,
                );
                transform_introspection_in_expr(
                    &mut generator.target,
                    target_fn,
                    recurse_into_scopes,
                );
                for if_clause in &mut generator.ifs {
                    transform_introspection_in_expr(if_clause, target_fn, recurse_into_scopes);
                }
            }
        }
        Expr::Generator(gen_expr) => {
            transform_introspection_in_expr(&mut gen_expr.elt, target_fn, recurse_into_scopes);
            for generator in &mut gen_expr.generators {
                transform_introspection_in_expr(
                    &mut generator.iter,
                    target_fn,
                    recurse_into_scopes,
                );
                transform_introspection_in_expr(
                    &mut generator.target,
                    target_fn,
                    recurse_into_scopes,
                );
                for if_clause in &mut generator.ifs {
                    transform_introspection_in_expr(if_clause, target_fn, recurse_into_scopes);
                }
            }
        }
        Expr::Compare(compare_expr) => {
            transform_introspection_in_expr(&mut compare_expr.left, target_fn, recurse_into_scopes);
            for comparator in &mut compare_expr.comparators {
                transform_introspection_in_expr(comparator, target_fn, recurse_into_scopes);
            }
        }
        Expr::BoolOp(bool_op_expr) => {
            for value in &mut bool_op_expr.values {
                transform_introspection_in_expr(value, target_fn, recurse_into_scopes);
            }
        }
        Expr::BinOp(bin_op_expr) => {
            transform_introspection_in_expr(&mut bin_op_expr.left, target_fn, recurse_into_scopes);
            transform_introspection_in_expr(&mut bin_op_expr.right, target_fn, recurse_into_scopes);
        }
        Expr::UnaryOp(unary_op_expr) => {
            transform_introspection_in_expr(
                &mut unary_op_expr.operand,
                target_fn,
                recurse_into_scopes,
            );
        }
        Expr::Tuple(tuple_expr) => {
            for elem in &mut tuple_expr.elts {
                transform_introspection_in_expr(elem, target_fn, recurse_into_scopes);
            }
        }
        Expr::Set(set_expr) => {
            for elem in &mut set_expr.elts {
                transform_introspection_in_expr(elem, target_fn, recurse_into_scopes);
            }
        }
        Expr::Slice(slice_expr) => {
            if let Some(ref mut lower) = slice_expr.lower {
                transform_introspection_in_expr(lower, target_fn, recurse_into_scopes);
            }
            if let Some(ref mut upper) = slice_expr.upper {
                transform_introspection_in_expr(upper, target_fn, recurse_into_scopes);
            }
            if let Some(ref mut step) = slice_expr.step {
                transform_introspection_in_expr(step, target_fn, recurse_into_scopes);
            }
        }
        Expr::Starred(starred_expr) => {
            transform_introspection_in_expr(
                &mut starred_expr.value,
                target_fn,
                recurse_into_scopes,
            );
        }
        Expr::Await(await_expr) => {
            transform_introspection_in_expr(&mut await_expr.value, target_fn, recurse_into_scopes);
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                transform_introspection_in_expr(value, target_fn, recurse_into_scopes);
            }
        }
        Expr::YieldFrom(yield_from) => {
            transform_introspection_in_expr(&mut yield_from.value, target_fn, recurse_into_scopes);
        }
        Expr::Named(named_expr) => {
            transform_introspection_in_expr(&mut named_expr.target, target_fn, recurse_into_scopes);
            transform_introspection_in_expr(&mut named_expr.value, target_fn, recurse_into_scopes);
        }
        // Base cases that don't need transformation
        _ => {}
    }
}

/// Unified function to transform module-level introspection calls in statements
/// For `locals()`: stops at function/class boundaries
/// For `globals()`: recurses into all contexts
fn transform_introspection_in_stmt(stmt: &mut Stmt, target_fn: &str, recurse_into_scopes: bool) {
    match stmt {
        Stmt::FunctionDef(func_def) if recurse_into_scopes => {
            // Only recurse into function bodies if allowed (for globals)
            for stmt in &mut func_def.body {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
        }
        Stmt::ClassDef(class_def) if recurse_into_scopes => {
            // Only recurse into class bodies if allowed (for globals)
            for stmt in &mut class_def.body {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
        }
        Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
            // Don't transform inside function/class definitions for locals()
        }
        Stmt::Expr(expr_stmt) => {
            transform_introspection_in_expr(&mut expr_stmt.value, target_fn, recurse_into_scopes);
        }
        Stmt::Assign(assign_stmt) => {
            transform_introspection_in_expr(&mut assign_stmt.value, target_fn, recurse_into_scopes);
            for target in &mut assign_stmt.targets {
                transform_introspection_in_expr(target, target_fn, recurse_into_scopes);
            }
        }
        Stmt::AnnAssign(ann_assign_stmt) => {
            if let Some(ref mut value) = ann_assign_stmt.value {
                transform_introspection_in_expr(value, target_fn, recurse_into_scopes);
            }
        }
        Stmt::AugAssign(aug_assign_stmt) => {
            transform_introspection_in_expr(
                &mut aug_assign_stmt.value,
                target_fn,
                recurse_into_scopes,
            );
        }
        Stmt::Return(return_stmt) => {
            if let Some(ref mut value) = return_stmt.value {
                transform_introspection_in_expr(value, target_fn, recurse_into_scopes);
            }
        }
        Stmt::Delete(delete_stmt) => {
            for target in &mut delete_stmt.targets {
                transform_introspection_in_expr(target, target_fn, recurse_into_scopes);
            }
        }
        Stmt::If(if_stmt) => {
            transform_introspection_in_expr(&mut if_stmt.test, target_fn, recurse_into_scopes);
            for stmt in &mut if_stmt.body {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(ref mut test_expr) = clause.test {
                    transform_introspection_in_expr(test_expr, target_fn, recurse_into_scopes);
                }
                for stmt in &mut clause.body {
                    transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
                }
            }
        }
        Stmt::For(for_stmt) => {
            transform_introspection_in_expr(&mut for_stmt.iter, target_fn, recurse_into_scopes);
            transform_introspection_in_expr(&mut for_stmt.target, target_fn, recurse_into_scopes);
            for stmt in &mut for_stmt.body {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
            for stmt in &mut for_stmt.orelse {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
        }
        Stmt::While(while_stmt) => {
            transform_introspection_in_expr(&mut while_stmt.test, target_fn, recurse_into_scopes);
            for stmt in &mut while_stmt.body {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
            for stmt in &mut while_stmt.orelse {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
        }
        Stmt::With(with_stmt) => {
            for item in &mut with_stmt.items {
                transform_introspection_in_expr(
                    &mut item.context_expr,
                    target_fn,
                    recurse_into_scopes,
                );
                if let Some(ref mut vars) = item.optional_vars {
                    transform_introspection_in_expr(vars, target_fn, recurse_into_scopes);
                }
            }
            for stmt in &mut with_stmt.body {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
        }
        Stmt::Match(match_stmt) => {
            transform_introspection_in_expr(
                &mut match_stmt.subject,
                target_fn,
                recurse_into_scopes,
            );
            for case in &mut match_stmt.cases {
                if let Some(ref mut guard) = case.guard {
                    transform_introspection_in_expr(guard, target_fn, recurse_into_scopes);
                }
                for stmt in &mut case.body {
                    transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
                }
            }
        }
        Stmt::Raise(raise_stmt) => {
            if let Some(ref mut exc) = raise_stmt.exc {
                transform_introspection_in_expr(exc, target_fn, recurse_into_scopes);
            }
            if let Some(ref mut cause) = raise_stmt.cause {
                transform_introspection_in_expr(cause, target_fn, recurse_into_scopes);
            }
        }
        Stmt::Try(try_stmt) => {
            for stmt in &mut try_stmt.body {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
            for handler in &mut try_stmt.handlers {
                let ExceptHandler::ExceptHandler(handler) = handler;
                if let Some(ref mut type_) = handler.type_ {
                    transform_introspection_in_expr(type_, target_fn, recurse_into_scopes);
                }
                for stmt in &mut handler.body {
                    transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
                }
            }
            for stmt in &mut try_stmt.orelse {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
            for stmt in &mut try_stmt.finalbody {
                transform_introspection_in_stmt(stmt, target_fn, recurse_into_scopes);
            }
        }
        Stmt::Assert(assert_stmt) => {
            transform_introspection_in_expr(&mut assert_stmt.test, target_fn, recurse_into_scopes);
            if let Some(ref mut msg) = assert_stmt.msg {
                transform_introspection_in_expr(msg, target_fn, recurse_into_scopes);
            }
        }
        // Statements that don't contain expressions or are not supported
        _ => {}
    }
}

/// Transform `globals()` calls in a statement
pub fn transform_globals_in_stmt(stmt: &mut Stmt) {
    // Use unified function with recursion enabled (globals recurses into all scopes)
    transform_introspection_in_stmt(stmt, "globals", true);
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
                let module_name_sanitized =
                    sanitize_module_name_for_identifier(&global_info.module_name);
                let var_name_sanitized = sanitize_var_name(var_name);
                let lifted_name = format!("_cribo_{module_name_sanitized}_{var_name_sanitized}");

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

    /// Get the lifted names mapping
    pub fn get_lifted_names(&self) -> &FxIndexMap<String, String> {
        &self.lifted_names
    }
}

/// Process wrapper module globals (matching original implementation)
pub fn process_wrapper_module_globals(
    params: &ProcessGlobalsParams,
    module_globals: &mut FxIndexMap<String, ModuleGlobalInfo>,
    all_lifted_declarations: &mut Vec<Stmt>,
) {
    // Get module ID from graph
    let Some(module) = params
        .semantic_ctx
        .graph
        .get_module_by_name(params.module_name)
    else {
        return;
    };

    let module_id = module.module_id;
    let global_info = params.semantic_ctx.semantic_bundler.analyze_module_globals(
        module_id,
        params.ast,
        params.module_name,
    );

    // Create GlobalsLifter and collect declarations
    if !global_info.global_declarations.is_empty() {
        let globals_lifter = GlobalsLifter::new(&global_info);
        all_lifted_declarations.extend_from_slice(&globals_lifter.lifted_declarations);
    }

    module_globals.insert(params.module_name.to_string(), global_info);
}

/// Transform `locals()` calls to `vars(__cribo_module)` in a statement
pub fn transform_locals_in_stmt(stmt: &mut Stmt) {
    // Use unified function with recursion disabled (locals stops at function/class boundaries)
    transform_introspection_in_stmt(stmt, "locals", false);
}
