use log::debug;
use ruff_python_ast::{
    AtomicNodeIndex, Comprehension, ExceptHandler, Expr, ExprContext, ExprFString, FString,
    FStringValue, InterpolatedElement, InterpolatedStringElement, InterpolatedStringElements, Stmt,
};

use crate::{
    ast_builder::expressions, code_generator::module_registry::sanitize_module_name_for_identifier,
    semantic_bundler::ModuleGlobalInfo, types::FxIndexMap,
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
}

/// Helper function to transform generators in comprehensions
fn transform_generators(
    generators: &mut [Comprehension],
    target_fn: &str,
    recurse_into_scopes: bool,
    module_var_name: Option<&str>,
) {
    for generator in generators {
        transform_introspection_in_expr(
            &mut generator.iter,
            target_fn,
            recurse_into_scopes,
            module_var_name,
        );
        transform_introspection_in_expr(
            &mut generator.target,
            target_fn,
            recurse_into_scopes,
            module_var_name,
        );
        for if_clause in &mut generator.ifs {
            transform_introspection_in_expr(
                if_clause,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
    }
}

/// Unified function to transform module-level introspection calls
/// For `locals()`: transforms to `vars(module_var)`, stops at function/class boundaries
/// For `globals()`: transforms to `module_var.__dict__`, recurses into all contexts
fn transform_introspection_in_expr(
    expr: &mut Expr,
    target_fn: &str,
    recurse_into_scopes: bool,
    module_var_name: Option<&str>,
) {
    match expr {
        Expr::Call(call_expr) => {
            // Check if this is the target introspection call
            if let Expr::Name(name_expr) = &*call_expr.func
                && name_expr.id.as_str() == target_fn
                && call_expr.arguments.args.is_empty()
                && call_expr.arguments.keywords.is_empty()
            {
                // Transform based on the target function
                if let Some(module_var) = module_var_name {
                    if target_fn == "locals" {
                        // Replace with vars(module_var)
                        *expr = expressions::call(
                            expressions::name("vars", ExprContext::Load),
                            vec![expressions::name(module_var, ExprContext::Load)],
                            vec![],
                        );
                    } else if target_fn == "globals" {
                        // Replace with module_var.__dict__
                        *expr = expressions::attribute(
                            expressions::name(module_var, ExprContext::Load),
                            "__dict__",
                            ExprContext::Load,
                        );
                    }
                } else {
                    // Fallback to using "self" if no module_var_name provided
                    if target_fn == "locals" {
                        // Replace with vars(self)
                        *expr = expressions::call(
                            expressions::name("vars", ExprContext::Load),
                            vec![expressions::name("self", ExprContext::Load)],
                            vec![],
                        );
                    } else if target_fn == "globals" {
                        // Replace with self.__dict__
                        *expr = expressions::attribute(
                            expressions::name("self", ExprContext::Load),
                            "__dict__",
                            ExprContext::Load,
                        );
                    }
                }
                return;
            }

            // Recursively transform in function and arguments
            transform_introspection_in_expr(
                &mut call_expr.func,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            for arg in &mut call_expr.arguments.args {
                transform_introspection_in_expr(
                    arg,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
            for keyword in &mut call_expr.arguments.keywords {
                transform_introspection_in_expr(
                    &mut keyword.value,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Expr::Lambda(lambda_expr) if recurse_into_scopes => {
            // Only recurse into lambda if allowed (for globals)
            transform_introspection_in_expr(
                &mut lambda_expr.body,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::Attribute(attr_expr) => {
            transform_introspection_in_expr(
                &mut attr_expr.value,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::Subscript(subscript_expr) => {
            transform_introspection_in_expr(
                &mut subscript_expr.value,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_introspection_in_expr(
                &mut subscript_expr.slice,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                transform_introspection_in_expr(
                    elem,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Expr::Dict(dict_expr) => {
            for item in &mut dict_expr.items {
                if let Some(ref mut key) = item.key {
                    transform_introspection_in_expr(
                        key,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
                transform_introspection_in_expr(
                    &mut item.value,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Expr::If(if_expr) => {
            transform_introspection_in_expr(
                &mut if_expr.test,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_introspection_in_expr(
                &mut if_expr.body,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_introspection_in_expr(
                &mut if_expr.orelse,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::ListComp(comp_expr) => {
            // List comprehensions: at module level they see module scope,
            // inside functions they see function scope
            transform_introspection_in_expr(
                &mut comp_expr.elt,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_generators(
                &mut comp_expr.generators,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::DictComp(comp_expr) => {
            // Dict comprehensions: at module level they see module scope,
            // inside functions they see function scope
            transform_introspection_in_expr(
                &mut comp_expr.key,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_introspection_in_expr(
                &mut comp_expr.value,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_generators(
                &mut comp_expr.generators,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::SetComp(comp_expr) => {
            // Set comprehensions: at module level they see module scope,
            // inside functions they see function scope
            transform_introspection_in_expr(
                &mut comp_expr.elt,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_generators(
                &mut comp_expr.generators,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::Generator(gen_expr) if recurse_into_scopes => {
            // Generator expressions have truly isolated scopes (like functions)
            // Only transform when doing globals() (recurse_into_scopes = true)
            transform_introspection_in_expr(
                &mut gen_expr.elt,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_generators(
                &mut gen_expr.generators,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::Generator(_) => {
            // Don't transform locals() inside generators at module level
            // They have their own isolated scope
        }
        Expr::Compare(compare_expr) => {
            transform_introspection_in_expr(
                &mut compare_expr.left,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            for comparator in &mut compare_expr.comparators {
                transform_introspection_in_expr(
                    comparator,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Expr::BoolOp(bool_op_expr) => {
            for value in &mut bool_op_expr.values {
                transform_introspection_in_expr(
                    value,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Expr::BinOp(bin_op_expr) => {
            transform_introspection_in_expr(
                &mut bin_op_expr.left,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_introspection_in_expr(
                &mut bin_op_expr.right,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::UnaryOp(unary_op_expr) => {
            transform_introspection_in_expr(
                &mut unary_op_expr.operand,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::Tuple(tuple_expr) => {
            for elem in &mut tuple_expr.elts {
                transform_introspection_in_expr(
                    elem,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Expr::Set(set_expr) => {
            for elem in &mut set_expr.elts {
                transform_introspection_in_expr(
                    elem,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Expr::Slice(slice_expr) => {
            if let Some(ref mut lower) = slice_expr.lower {
                transform_introspection_in_expr(
                    lower,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
            if let Some(ref mut upper) = slice_expr.upper {
                transform_introspection_in_expr(
                    upper,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
            if let Some(ref mut step) = slice_expr.step {
                transform_introspection_in_expr(
                    step,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Expr::Starred(starred_expr) => {
            transform_introspection_in_expr(
                &mut starred_expr.value,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::Await(await_expr) => {
            transform_introspection_in_expr(
                &mut await_expr.value,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                transform_introspection_in_expr(
                    value,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Expr::YieldFrom(yield_from) => {
            transform_introspection_in_expr(
                &mut yield_from.value,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::Named(named_expr) => {
            transform_introspection_in_expr(
                &mut named_expr.target,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_introspection_in_expr(
                &mut named_expr.value,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Expr::FString(fstring_expr) => {
            // Transform expressions within f-string interpolations
            // We need to rebuild the f-string if any expressions are transformed
            let mut transformed_elements = Vec::new();
            let mut any_transformed = false;

            for element in fstring_expr.value.elements() {
                match element {
                    InterpolatedStringElement::Literal(lit_elem) => {
                        // Literal strings don't need transformation
                        transformed_elements
                            .push(InterpolatedStringElement::Literal(lit_elem.clone()));
                    }
                    InterpolatedStringElement::Interpolation(expr_elem) => {
                        // Transform the embedded expression
                        let mut new_expr = expr_elem.expression.clone();
                        let old_expr = new_expr.clone();
                        transform_introspection_in_expr(
                            &mut new_expr,
                            target_fn,
                            recurse_into_scopes,
                            module_var_name,
                        );

                        if !matches!(&new_expr, other if other == &old_expr) {
                            any_transformed = true;
                        }

                        let new_element = InterpolatedElement {
                            node_index: AtomicNodeIndex::dummy(),
                            expression: new_expr,
                            debug_text: expr_elem.debug_text.clone(),
                            conversion: expr_elem.conversion,
                            format_spec: expr_elem.format_spec.clone(),
                            range: expr_elem.range,
                        };
                        transformed_elements
                            .push(InterpolatedStringElement::Interpolation(new_element));
                    }
                }
            }

            // If any expressions were transformed, rebuild the f-string
            if any_transformed {
                let original_flags =
                    crate::ast_builder::expressions::get_fstring_flags(&fstring_expr.value);

                let new_fstring = FString {
                    node_index: AtomicNodeIndex::dummy(),
                    elements: InterpolatedStringElements::from(transformed_elements),
                    range: fstring_expr.range,
                    flags: original_flags,
                };

                let new_value = FStringValue::single(new_fstring);

                *expr = Expr::FString(ExprFString {
                    node_index: AtomicNodeIndex::dummy(),
                    value: new_value,
                    range: fstring_expr.range,
                });
            }
        }
        // Base cases that don't need transformation
        _ => {}
    }
}

/// Unified function to transform module-level introspection calls in statements
/// For `locals()`: stops at function/class boundaries
/// For `globals()`: recurses into all contexts
fn transform_introspection_in_stmt(
    stmt: &mut Stmt,
    target_fn: &str,
    recurse_into_scopes: bool,
    module_var_name: Option<&str>,
) {
    match stmt {
        Stmt::FunctionDef(func_def) => {
            // Decorators are evaluated at definition time in the enclosing scope
            for decorator in &mut func_def.decorator_list {
                transform_introspection_in_expr(
                    &mut decorator.expression,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }

            // Return type annotation is evaluated at definition time
            if let Some(ref mut returns) = func_def.returns {
                transform_introspection_in_expr(
                    returns,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }

            // Parameter defaults are evaluated at definition time
            for param in func_def
                .parameters
                .posonlyargs
                .iter_mut()
                .chain(func_def.parameters.args.iter_mut())
                .chain(func_def.parameters.kwonlyargs.iter_mut())
            {
                if let Some(ref mut default) = param.default {
                    transform_introspection_in_expr(
                        default,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
                // Note: parameter annotations are also evaluated at definition time
                if let Some(ref mut annotation) = param.parameter.annotation {
                    transform_introspection_in_expr(
                        annotation,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
            }

            // Only recurse into function body if allowed
            if recurse_into_scopes {
                for stmt in &mut func_def.body {
                    transform_introspection_in_stmt(
                        stmt,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
            }
        }
        Stmt::ClassDef(class_def) => {
            // Decorators are evaluated at definition time in the enclosing scope
            for decorator in &mut class_def.decorator_list {
                transform_introspection_in_expr(
                    &mut decorator.expression,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }

            // Base classes and keywords are evaluated at definition time
            if let Some(ref mut arguments) = class_def.arguments {
                for base in &mut arguments.args {
                    transform_introspection_in_expr(
                        base,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
                for keyword in &mut arguments.keywords {
                    transform_introspection_in_expr(
                        &mut keyword.value,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
            }

            // Only recurse into class body if allowed
            if recurse_into_scopes {
                for stmt in &mut class_def.body {
                    transform_introspection_in_stmt(
                        stmt,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
            }
        }
        Stmt::Expr(expr_stmt) => {
            transform_introspection_in_expr(
                &mut expr_stmt.value,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Stmt::Assign(assign_stmt) => {
            transform_introspection_in_expr(
                &mut assign_stmt.value,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            for target in &mut assign_stmt.targets {
                transform_introspection_in_expr(
                    target,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Stmt::AnnAssign(ann_assign_stmt) => {
            if let Some(ref mut value) = ann_assign_stmt.value {
                transform_introspection_in_expr(
                    value,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Stmt::AugAssign(aug_assign_stmt) => {
            transform_introspection_in_expr(
                &mut aug_assign_stmt.value,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
        }
        Stmt::Return(return_stmt) => {
            if let Some(ref mut value) = return_stmt.value {
                transform_introspection_in_expr(
                    value,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Stmt::Delete(delete_stmt) => {
            for target in &mut delete_stmt.targets {
                transform_introspection_in_expr(
                    target,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Stmt::If(if_stmt) => {
            transform_introspection_in_expr(
                &mut if_stmt.test,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            for stmt in &mut if_stmt.body {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(ref mut test_expr) = clause.test {
                    transform_introspection_in_expr(
                        test_expr,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
                for stmt in &mut clause.body {
                    transform_introspection_in_stmt(
                        stmt,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
            }
        }
        Stmt::For(for_stmt) => {
            transform_introspection_in_expr(
                &mut for_stmt.iter,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            transform_introspection_in_expr(
                &mut for_stmt.target,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            for stmt in &mut for_stmt.body {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
            for stmt in &mut for_stmt.orelse {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Stmt::While(while_stmt) => {
            transform_introspection_in_expr(
                &mut while_stmt.test,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            for stmt in &mut while_stmt.body {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
            for stmt in &mut while_stmt.orelse {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Stmt::With(with_stmt) => {
            for item in &mut with_stmt.items {
                transform_introspection_in_expr(
                    &mut item.context_expr,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
                if let Some(ref mut vars) = item.optional_vars {
                    transform_introspection_in_expr(
                        vars,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
            }
            for stmt in &mut with_stmt.body {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Stmt::Match(match_stmt) => {
            transform_introspection_in_expr(
                &mut match_stmt.subject,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            for case in &mut match_stmt.cases {
                if let Some(ref mut guard) = case.guard {
                    transform_introspection_in_expr(
                        guard,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
                for stmt in &mut case.body {
                    transform_introspection_in_stmt(
                        stmt,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
            }
        }
        Stmt::Raise(raise_stmt) => {
            if let Some(ref mut exc) = raise_stmt.exc {
                transform_introspection_in_expr(
                    exc,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
            if let Some(ref mut cause) = raise_stmt.cause {
                transform_introspection_in_expr(
                    cause,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Stmt::Try(try_stmt) => {
            for stmt in &mut try_stmt.body {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
            for handler in &mut try_stmt.handlers {
                let ExceptHandler::ExceptHandler(handler) = handler;
                if let Some(ref mut type_) = handler.type_ {
                    transform_introspection_in_expr(
                        type_,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
                for stmt in &mut handler.body {
                    transform_introspection_in_stmt(
                        stmt,
                        target_fn,
                        recurse_into_scopes,
                        module_var_name,
                    );
                }
            }
            for stmt in &mut try_stmt.orelse {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
            for stmt in &mut try_stmt.finalbody {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        Stmt::Assert(assert_stmt) => {
            transform_introspection_in_expr(
                &mut assert_stmt.test,
                target_fn,
                recurse_into_scopes,
                module_var_name,
            );
            if let Some(ref mut msg) = assert_stmt.msg {
                transform_introspection_in_expr(
                    msg,
                    target_fn,
                    recurse_into_scopes,
                    module_var_name,
                );
            }
        }
        // Statements that don't contain expressions or are not supported
        _ => {}
    }
}

/// Transform `globals()` calls in a statement
pub fn transform_globals_in_stmt(stmt: &mut Stmt, module_var_name: &str) {
    // Use unified function with recursion enabled (globals recurses into all scopes)
    transform_introspection_in_stmt(stmt, "globals", true, Some(module_var_name));
}

impl GlobalsLifter {
    pub fn new(global_info: &ModuleGlobalInfo) -> Self {
        let mut lifted_names = FxIndexMap::default();

        debug!("GlobalsLifter::new for module: {}", global_info.module_name);
        debug!("Module level vars: {:?}", global_info.module_level_vars);
        debug!(
            "Global declarations: {:?}",
            global_info.global_declarations.keys().collect::<Vec<_>>()
        );

        // Generate lifted names and declarations for all variables that are referenced with
        // global statements. Use the broader set that includes imports and functions too.
        let candidates = if global_info.liftable_vars.is_empty() {
            &global_info.module_level_vars
        } else {
            &global_info.liftable_vars
        };

        for var_name in candidates {
            // Only lift variables that are actually used with global statements
            if global_info.global_declarations.contains_key(var_name) {
                let module_name_sanitized =
                    sanitize_module_name_for_identifier(&global_info.module_name);
                let var_name_sanitized = sanitize_var_name(var_name);
                let lifted_name = format!("_cribo_{module_name_sanitized}_{var_name_sanitized}");

                debug!("Creating lifted declaration for {var_name} -> {lifted_name}");

                lifted_names.insert(var_name.clone(), lifted_name.clone());
            }
        }

        Self { lifted_names }
    }

    /// Get the lifted names mapping
    pub fn get_lifted_names(&self) -> &FxIndexMap<String, String> {
        &self.lifted_names
    }
}

/// Transform `locals()` calls to `vars(module_var)` in a statement
pub fn transform_locals_in_stmt(stmt: &mut Stmt, module_var_name: &str) {
    // Use unified function with recursion disabled (locals stops at function/class boundaries)
    transform_introspection_in_stmt(stmt, "locals", false, Some(module_var_name));
}
