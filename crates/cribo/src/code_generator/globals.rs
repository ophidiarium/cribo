use log::debug;
use ruff_python_ast::{
    AtomicNodeIndex, Comprehension, ExceptHandler, Expr, ExprContext, ExprFString, FString,
    FStringValue, InterpolatedElement, InterpolatedStringElement, InterpolatedStringElements, Stmt,
};
use rustc_hash::FxHashSet;

use crate::{
    ast_builder::{expressions, statements},
    code_generator::{
        context::ProcessGlobalsParams,
        module_registry::{MODULE_VAR, sanitize_module_name_for_identifier},
    },
    semantic_bundler::ModuleGlobalInfo,
    types::FxIndexMap,
};

/// Collect assignments to introspection function names ("locals" and "globals")
/// that would shadow the builtin functions
fn collect_shadowed_introspection_names(stmts: &[Stmt], shadowed_names: &mut FxHashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Assign(assign) => {
                // Collect assignment targets that shadow introspection functions
                for target in &assign.targets {
                    if let Expr::Name(name) = target {
                        let name_str = name.id.as_str();
                        if name_str == "locals" || name_str == "globals" {
                            debug!(
                                "Found assignment that shadows introspection function: {name_str}"
                            );
                            shadowed_names.insert(name.id.to_string());
                        }
                    }
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                // Collect annotated assignment targets
                if let Expr::Name(name) = ann_assign.target.as_ref() {
                    let name_str = name.id.as_str();
                    if name_str == "locals" || name_str == "globals" {
                        debug!(
                            "Found annotated assignment that shadows introspection function: {name_str}"
                        );
                        shadowed_names.insert(name.id.to_string());
                    }
                }
            }
            Stmt::For(for_stmt) => {
                // Collect for loop targets
                if let Expr::Name(name) = for_stmt.target.as_ref() {
                    let name_str = name.id.as_str();
                    if name_str == "locals" || name_str == "globals" {
                        debug!(
                            "Found for loop target that shadows introspection function: {name_str}"
                        );
                        shadowed_names.insert(name.id.to_string());
                    }
                }
                // Recursively collect from body
                collect_shadowed_introspection_names(&for_stmt.body, shadowed_names);
                collect_shadowed_introspection_names(&for_stmt.orelse, shadowed_names);
            }
            Stmt::If(if_stmt) => {
                // Recursively collect from branches
                collect_shadowed_introspection_names(&if_stmt.body, shadowed_names);
                for clause in &if_stmt.elif_else_clauses {
                    collect_shadowed_introspection_names(&clause.body, shadowed_names);
                }
            }
            Stmt::While(while_stmt) => {
                collect_shadowed_introspection_names(&while_stmt.body, shadowed_names);
                collect_shadowed_introspection_names(&while_stmt.orelse, shadowed_names);
            }
            Stmt::With(with_stmt) => {
                // Collect with statement targets
                for item in &with_stmt.items {
                    if let Some(ref optional_vars) = item.optional_vars
                        && let Expr::Name(name) = optional_vars.as_ref()
                    {
                        let name_str = name.id.as_str();
                        if name_str == "locals" || name_str == "globals" {
                            debug!(
                                "Found with statement target that shadows introspection function: {name_str}"
                            );
                            shadowed_names.insert(name.id.to_string());
                        }
                    }
                }
                collect_shadowed_introspection_names(&with_stmt.body, shadowed_names);
            }
            Stmt::Try(try_stmt) => {
                collect_shadowed_introspection_names(&try_stmt.body, shadowed_names);
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    // Collect exception name if present
                    if let Some(ref name) = eh.name {
                        let name_str = name.as_str();
                        if name_str == "locals" || name_str == "globals" {
                            debug!(
                                "Found exception handler name that shadows introspection function: {name_str}"
                            );
                            shadowed_names.insert(name.to_string());
                        }
                    }
                    collect_shadowed_introspection_names(&eh.body, shadowed_names);
                }
                collect_shadowed_introspection_names(&try_stmt.orelse, shadowed_names);
                collect_shadowed_introspection_names(&try_stmt.finalbody, shadowed_names);
            }
            Stmt::FunctionDef(func_def) => {
                // Function definitions create local names
                let name_str = func_def.name.as_str();
                if name_str == "locals" || name_str == "globals" {
                    debug!(
                        "Found function definition that shadows introspection function: {name_str}"
                    );
                    shadowed_names.insert(func_def.name.to_string());
                }
            }
            Stmt::ClassDef(class_def) => {
                // Class definitions create local names
                let name_str = class_def.name.as_str();
                if name_str == "locals" || name_str == "globals" {
                    debug!(
                        "Found class definition that shadows introspection function: {name_str}"
                    );
                    shadowed_names.insert(class_def.name.to_string());
                }
            }
            _ => {
                // Other statements don't create variables that could shadow introspection functions
            }
        }
    }
}

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

/// Helper function to transform generators in comprehensions
fn transform_generators(
    generators: &mut [Comprehension],
    target_fn: &str,
    recurse_into_scopes: bool,
    shadowed_names: &FxHashSet<String>,
) {
    for generator in generators {
        transform_introspection_in_expr(
            &mut generator.iter,
            target_fn,
            recurse_into_scopes,
            shadowed_names,
        );
        transform_introspection_in_expr(
            &mut generator.target,
            target_fn,
            recurse_into_scopes,
            shadowed_names,
        );
        for if_clause in &mut generator.ifs {
            transform_introspection_in_expr(
                if_clause,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
    }
}

/// Unified function to transform module-level introspection calls
/// For `locals()`: transforms to `vars(__cribo_module)`, stops at function/class boundaries
/// For `globals()`: transforms to `__cribo_module.__dict__`, recurses into all contexts
fn transform_introspection_in_expr(
    expr: &mut Expr,
    target_fn: &str,
    recurse_into_scopes: bool,
    shadowed_names: &FxHashSet<String>,
) {
    match expr {
        Expr::Call(call_expr) => {
            // Check if this is the target introspection call
            if let Expr::Name(name_expr) = &*call_expr.func
                && name_expr.id.as_str() == target_fn
                && call_expr.arguments.args.is_empty()
                && call_expr.arguments.keywords.is_empty()
            {
                // Skip transformation if the name is shadowed
                if shadowed_names.contains(target_fn) {
                    debug!(
                        "Skipping transformation of shadowed introspection function: {target_fn}"
                    );
                    return;
                }

                // Transform based on the target function
                if target_fn == "locals" {
                    // Replace with vars(__cribo_module)
                    *expr = expressions::call(
                        expressions::name("vars", ExprContext::Load),
                        vec![expressions::name(MODULE_VAR, ExprContext::Load)],
                        vec![],
                    );
                } else if target_fn == "globals" {
                    // Replace with __cribo_module.__dict__
                    *expr = expressions::attribute(
                        expressions::name(MODULE_VAR, ExprContext::Load),
                        "__dict__",
                        ExprContext::Load,
                    );
                }
                return;
            }

            // Recursively transform in function and arguments
            transform_introspection_in_expr(
                &mut call_expr.func,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            for arg in &mut call_expr.arguments.args {
                transform_introspection_in_expr(
                    arg,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
            for keyword in &mut call_expr.arguments.keywords {
                transform_introspection_in_expr(
                    &mut keyword.value,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Expr::Lambda(lambda_expr) if recurse_into_scopes => {
            // Only recurse into lambda if allowed (for globals)
            transform_introspection_in_expr(
                &mut lambda_expr.body,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::Attribute(attr_expr) => {
            transform_introspection_in_expr(
                &mut attr_expr.value,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::Subscript(subscript_expr) => {
            transform_introspection_in_expr(
                &mut subscript_expr.value,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_introspection_in_expr(
                &mut subscript_expr.slice,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                transform_introspection_in_expr(
                    elem,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
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
                        shadowed_names,
                    );
                }
                transform_introspection_in_expr(
                    &mut item.value,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Expr::If(if_expr) => {
            transform_introspection_in_expr(
                &mut if_expr.test,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_introspection_in_expr(
                &mut if_expr.body,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_introspection_in_expr(
                &mut if_expr.orelse,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::ListComp(comp_expr) => {
            // List comprehensions: at module level they see module scope,
            // inside functions they see function scope
            transform_introspection_in_expr(
                &mut comp_expr.elt,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_generators(
                &mut comp_expr.generators,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::DictComp(comp_expr) => {
            // Dict comprehensions: at module level they see module scope,
            // inside functions they see function scope
            transform_introspection_in_expr(
                &mut comp_expr.key,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_introspection_in_expr(
                &mut comp_expr.value,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_generators(
                &mut comp_expr.generators,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::SetComp(comp_expr) => {
            // Set comprehensions: at module level they see module scope,
            // inside functions they see function scope
            transform_introspection_in_expr(
                &mut comp_expr.elt,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_generators(
                &mut comp_expr.generators,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::Generator(gen_expr) if recurse_into_scopes => {
            // Generator expressions have truly isolated scopes (like functions)
            // Only transform when doing globals() (recurse_into_scopes = true)
            transform_introspection_in_expr(
                &mut gen_expr.elt,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_generators(
                &mut gen_expr.generators,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
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
                shadowed_names,
            );
            for comparator in &mut compare_expr.comparators {
                transform_introspection_in_expr(
                    comparator,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Expr::BoolOp(bool_op_expr) => {
            for value in &mut bool_op_expr.values {
                transform_introspection_in_expr(
                    value,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Expr::BinOp(bin_op_expr) => {
            transform_introspection_in_expr(
                &mut bin_op_expr.left,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_introspection_in_expr(
                &mut bin_op_expr.right,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::UnaryOp(unary_op_expr) => {
            transform_introspection_in_expr(
                &mut unary_op_expr.operand,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::Tuple(tuple_expr) => {
            for elem in &mut tuple_expr.elts {
                transform_introspection_in_expr(
                    elem,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Expr::Set(set_expr) => {
            for elem in &mut set_expr.elts {
                transform_introspection_in_expr(
                    elem,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Expr::Slice(slice_expr) => {
            if let Some(ref mut lower) = slice_expr.lower {
                transform_introspection_in_expr(
                    lower,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
            if let Some(ref mut upper) = slice_expr.upper {
                transform_introspection_in_expr(
                    upper,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
            if let Some(ref mut step) = slice_expr.step {
                transform_introspection_in_expr(
                    step,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Expr::Starred(starred_expr) => {
            transform_introspection_in_expr(
                &mut starred_expr.value,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::Await(await_expr) => {
            transform_introspection_in_expr(
                &mut await_expr.value,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                transform_introspection_in_expr(
                    value,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Expr::YieldFrom(yield_from) => {
            transform_introspection_in_expr(
                &mut yield_from.value,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Expr::Named(named_expr) => {
            transform_introspection_in_expr(
                &mut named_expr.target,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_introspection_in_expr(
                &mut named_expr.value,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
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
                            shadowed_names,
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
    shadowed_names: &FxHashSet<String>,
) {
    match stmt {
        Stmt::FunctionDef(func_def) => {
            // Decorators are evaluated at definition time in the enclosing scope
            for decorator in &mut func_def.decorator_list {
                transform_introspection_in_expr(
                    &mut decorator.expression,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }

            // Return type annotation is evaluated at definition time
            if let Some(ref mut returns) = func_def.returns {
                transform_introspection_in_expr(
                    returns,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
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
                        shadowed_names,
                    );
                }
                // Note: parameter annotations are also evaluated at definition time
                if let Some(ref mut annotation) = param.parameter.annotation {
                    transform_introspection_in_expr(
                        annotation,
                        target_fn,
                        recurse_into_scopes,
                        shadowed_names,
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
                        shadowed_names,
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
                    shadowed_names,
                );
            }

            // Base classes and keywords are evaluated at definition time
            if let Some(ref mut arguments) = class_def.arguments {
                for base in &mut arguments.args {
                    transform_introspection_in_expr(
                        base,
                        target_fn,
                        recurse_into_scopes,
                        shadowed_names,
                    );
                }
                for keyword in &mut arguments.keywords {
                    transform_introspection_in_expr(
                        &mut keyword.value,
                        target_fn,
                        recurse_into_scopes,
                        shadowed_names,
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
                        shadowed_names,
                    );
                }
            }
        }
        Stmt::Expr(expr_stmt) => {
            transform_introspection_in_expr(
                &mut expr_stmt.value,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Stmt::Assign(assign_stmt) => {
            transform_introspection_in_expr(
                &mut assign_stmt.value,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            for target in &mut assign_stmt.targets {
                transform_introspection_in_expr(
                    target,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Stmt::AnnAssign(ann_assign_stmt) => {
            if let Some(ref mut value) = ann_assign_stmt.value {
                transform_introspection_in_expr(
                    value,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Stmt::AugAssign(aug_assign_stmt) => {
            transform_introspection_in_expr(
                &mut aug_assign_stmt.value,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
        }
        Stmt::Return(return_stmt) => {
            if let Some(ref mut value) = return_stmt.value {
                transform_introspection_in_expr(
                    value,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Stmt::Delete(delete_stmt) => {
            for target in &mut delete_stmt.targets {
                transform_introspection_in_expr(
                    target,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Stmt::If(if_stmt) => {
            transform_introspection_in_expr(
                &mut if_stmt.test,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            for stmt in &mut if_stmt.body {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(ref mut test_expr) = clause.test {
                    transform_introspection_in_expr(
                        test_expr,
                        target_fn,
                        recurse_into_scopes,
                        shadowed_names,
                    );
                }
                for stmt in &mut clause.body {
                    transform_introspection_in_stmt(
                        stmt,
                        target_fn,
                        recurse_into_scopes,
                        shadowed_names,
                    );
                }
            }
        }
        Stmt::For(for_stmt) => {
            transform_introspection_in_expr(
                &mut for_stmt.iter,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            transform_introspection_in_expr(
                &mut for_stmt.target,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            for stmt in &mut for_stmt.body {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
            for stmt in &mut for_stmt.orelse {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Stmt::While(while_stmt) => {
            transform_introspection_in_expr(
                &mut while_stmt.test,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            for stmt in &mut while_stmt.body {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
            for stmt in &mut while_stmt.orelse {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Stmt::With(with_stmt) => {
            for item in &mut with_stmt.items {
                transform_introspection_in_expr(
                    &mut item.context_expr,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
                if let Some(ref mut vars) = item.optional_vars {
                    transform_introspection_in_expr(
                        vars,
                        target_fn,
                        recurse_into_scopes,
                        shadowed_names,
                    );
                }
            }
            for stmt in &mut with_stmt.body {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Stmt::Match(match_stmt) => {
            transform_introspection_in_expr(
                &mut match_stmt.subject,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            for case in &mut match_stmt.cases {
                if let Some(ref mut guard) = case.guard {
                    transform_introspection_in_expr(
                        guard,
                        target_fn,
                        recurse_into_scopes,
                        shadowed_names,
                    );
                }
                for stmt in &mut case.body {
                    transform_introspection_in_stmt(
                        stmt,
                        target_fn,
                        recurse_into_scopes,
                        shadowed_names,
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
                    shadowed_names,
                );
            }
            if let Some(ref mut cause) = raise_stmt.cause {
                transform_introspection_in_expr(
                    cause,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Stmt::Try(try_stmt) => {
            for stmt in &mut try_stmt.body {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
            for handler in &mut try_stmt.handlers {
                let ExceptHandler::ExceptHandler(handler) = handler;
                if let Some(ref mut type_) = handler.type_ {
                    transform_introspection_in_expr(
                        type_,
                        target_fn,
                        recurse_into_scopes,
                        shadowed_names,
                    );
                }
                for stmt in &mut handler.body {
                    transform_introspection_in_stmt(
                        stmt,
                        target_fn,
                        recurse_into_scopes,
                        shadowed_names,
                    );
                }
            }
            for stmt in &mut try_stmt.orelse {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
            for stmt in &mut try_stmt.finalbody {
                transform_introspection_in_stmt(
                    stmt,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        Stmt::Assert(assert_stmt) => {
            transform_introspection_in_expr(
                &mut assert_stmt.test,
                target_fn,
                recurse_into_scopes,
                shadowed_names,
            );
            if let Some(ref mut msg) = assert_stmt.msg {
                transform_introspection_in_expr(
                    msg,
                    target_fn,
                    recurse_into_scopes,
                    shadowed_names,
                );
            }
        }
        // Statements that don't contain expressions or are not supported
        _ => {}
    }
}

/// Transform `globals()` calls in a statement
pub fn transform_globals_in_stmt(stmt: &mut Stmt) {
    // Collect shadowed names first from just this statement
    let mut shadowed_names = FxHashSet::default();
    collect_shadowed_introspection_names(&[stmt.clone()], &mut shadowed_names);

    // Use unified function with recursion enabled (globals recurses into all scopes)
    transform_introspection_in_stmt(stmt, "globals", true, &shadowed_names);
}

/// Transform `globals()` calls in a list of statements with proper shadowing detection
/// Add assignments from a single statement to the shadowed names set
fn add_statement_shadowed_names(stmt: &Stmt, shadowed_names: &mut FxHashSet<String>) {
    match stmt {
        Stmt::Assign(assign) => {
            // Collect assignment targets that shadow introspection functions
            for target in &assign.targets {
                if let Expr::Name(name) = target {
                    let name_str = name.id.as_str();
                    if name_str == "locals" || name_str == "globals" {
                        debug!("Found assignment that shadows introspection function: {name_str}");
                        shadowed_names.insert(name.id.to_string());
                    }
                }
            }
        }
        Stmt::AnnAssign(ann_assign) => {
            // Collect annotated assignment targets
            if let Expr::Name(name) = ann_assign.target.as_ref() {
                let name_str = name.id.as_str();
                if name_str == "locals" || name_str == "globals" {
                    debug!(
                        "Found annotated assignment that shadows introspection function: {name_str}"
                    );
                    shadowed_names.insert(name.id.to_string());
                }
            }
        }
        Stmt::For(for_stmt) => {
            // Collect for loop targets
            if let Expr::Name(name) = for_stmt.target.as_ref() {
                let name_str = name.id.as_str();
                if name_str == "locals" || name_str == "globals" {
                    debug!("Found for loop target that shadows introspection function: {name_str}");
                    shadowed_names.insert(name.id.to_string());
                }
            }
        }
        Stmt::With(with_stmt) => {
            // Collect with statement targets
            for item in &with_stmt.items {
                if let Some(ref optional_vars) = item.optional_vars
                    && let Expr::Name(name) = optional_vars.as_ref()
                {
                    let name_str = name.id.as_str();
                    if name_str == "locals" || name_str == "globals" {
                        debug!(
                            "Found with statement target that shadows introspection function: {name_str}"
                        );
                        shadowed_names.insert(name.id.to_string());
                    }
                }
            }
        }
        Stmt::Try(try_stmt) => {
            for handler in &try_stmt.handlers {
                let ExceptHandler::ExceptHandler(eh) = handler;
                // Collect exception name if present
                if let Some(ref name) = eh.name {
                    let name_str = name.as_str();
                    if name_str == "locals" || name_str == "globals" {
                        debug!(
                            "Found exception handler name that shadows introspection function: {name_str}"
                        );
                        shadowed_names.insert(name.to_string());
                    }
                }
            }
        }
        Stmt::FunctionDef(func_def) => {
            // Function definitions create local names
            let name_str = func_def.name.as_str();
            if name_str == "locals" || name_str == "globals" {
                debug!("Found function definition that shadows introspection function: {name_str}");
                shadowed_names.insert(func_def.name.to_string());
            }
        }
        Stmt::ClassDef(class_def) => {
            // Class definitions create local names
            let name_str = class_def.name.as_str();
            if name_str == "locals" || name_str == "globals" {
                debug!("Found class definition that shadows introspection function: {name_str}");
                shadowed_names.insert(class_def.name.to_string());
            }
        }
        _ => {
            // Other statements don't create variables that could shadow introspection functions
        }
    }
}

/// Transform `globals()` calls in a list of statements with proper sequential shadowing detection
pub fn transform_globals_in_stmts(stmts: &mut [Stmt]) {
    debug!(
        "transform_globals_in_stmts called with {} statements",
        stmts.len()
    );
    // Process statements sequentially, tracking shadowed names as we go
    let mut shadowed_names = FxHashSet::default();

    for stmt in stmts {
        // Transform the statement with current shadowed names
        transform_introspection_in_stmt(stmt, "globals", true, &shadowed_names);

        // Add any new shadowed names from this statement
        add_statement_shadowed_names(stmt, &mut shadowed_names);
    }
}

/// Transform `locals()` calls in a list of statements with proper sequential shadowing detection
pub fn transform_locals_in_stmts(stmts: &mut [Stmt]) {
    debug!(
        "transform_locals_in_stmts called with {} statements",
        stmts.len()
    );
    // Process statements sequentially, tracking shadowed names as we go
    let mut shadowed_names = FxHashSet::default();

    for stmt in stmts {
        // Transform the statement with current shadowed names
        transform_introspection_in_stmt(stmt, "locals", false, &shadowed_names);

        // Add any new shadowed names from this statement
        add_statement_shadowed_names(stmt, &mut shadowed_names);
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
