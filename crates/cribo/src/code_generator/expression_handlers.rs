//! Expression handling utilities for code generation
//!
//! This module contains functions for creating, analyzing, and transforming
//! `rustpython_parser::ast::Expr` nodes during the bundling process.

use log::debug;
use ruff_python_ast::{
    AtomicNodeIndex, ExceptHandler, Expr, ExprAttribute, ExprContext, Identifier, Stmt, StmtAssign,
    StmtClassDef, StmtFunctionDef,
};
use ruff_text_size::TextRange;

use super::bundler::Bundler;
use crate::{
    ast_builder::expressions,
    types::{FxIndexMap, FxIndexSet},
};

/// Extract attribute path from expression (e.g., "foo.bar.baz" from foo.bar.baz)
pub(super) fn extract_attribute_path(attr: &ExprAttribute) -> String {
    let mut parts = Vec::new();
    let mut current = attr;

    loop {
        parts.push(current.attr.as_str());
        match &*current.value {
            Expr::Attribute(parent_attr) => current = parent_attr,
            Expr::Name(name) => {
                parts.push(name.id.as_str());
                break;
            }
            _ => break,
        }
    }

    parts.reverse();
    parts.join(".")
}

/// Check if two expressions are equal
pub(super) fn expr_equals(expr1: &Expr, expr2: &Expr) -> bool {
    match (expr1, expr2) {
        (Expr::Name(n1), Expr::Name(n2)) => n1.id == n2.id,
        (Expr::Attribute(a1), Expr::Attribute(a2)) => {
            a1.attr == a2.attr && expr_equals(&a1.value, &a2.value)
        }
        _ => false,
    }
}

/// Resolve import aliases in expression
pub(super) fn resolve_import_aliases_in_expr(
    expr: &mut Expr,
    import_aliases: &FxIndexMap<String, String>,
) {
    match expr {
        Expr::Name(name) => {
            if let Some(canonical_name) = import_aliases.get(name.id.as_str()) {
                name.id = canonical_name.clone().into();
            }
        }
        Expr::Attribute(attr) => {
            resolve_import_aliases_in_expr(&mut attr.value, import_aliases);
        }
        Expr::Call(call) => {
            resolve_import_aliases_in_expr(&mut call.func, import_aliases);
            for arg in &mut call.arguments.args {
                resolve_import_aliases_in_expr(arg, import_aliases);
            }
            for keyword in &mut call.arguments.keywords {
                resolve_import_aliases_in_expr(&mut keyword.value, import_aliases);
            }
        }
        Expr::Subscript(sub) => {
            resolve_import_aliases_in_expr(&mut sub.value, import_aliases);
            resolve_import_aliases_in_expr(&mut sub.slice, import_aliases);
        }
        Expr::List(list) => {
            for elem in &mut list.elts {
                resolve_import_aliases_in_expr(elem, import_aliases);
            }
        }
        Expr::Tuple(tuple) => {
            for elem in &mut tuple.elts {
                resolve_import_aliases_in_expr(elem, import_aliases);
            }
        }
        Expr::Set(set) => {
            for elem in &mut set.elts {
                resolve_import_aliases_in_expr(elem, import_aliases);
            }
        }
        Expr::Dict(dict) => {
            for item in &mut dict.items {
                if let Some(ref mut key) = item.key {
                    resolve_import_aliases_in_expr(key, import_aliases);
                }
                resolve_import_aliases_in_expr(&mut item.value, import_aliases);
            }
        }
        Expr::ListComp(comp) => {
            resolve_import_aliases_in_expr(&mut comp.elt, import_aliases);
            for generator in &mut comp.generators {
                resolve_import_aliases_in_expr(&mut generator.iter, import_aliases);
                for if_clause in &mut generator.ifs {
                    resolve_import_aliases_in_expr(if_clause, import_aliases);
                }
            }
        }
        Expr::SetComp(comp) => {
            resolve_import_aliases_in_expr(&mut comp.elt, import_aliases);
            for generator in &mut comp.generators {
                resolve_import_aliases_in_expr(&mut generator.iter, import_aliases);
                for if_clause in &mut generator.ifs {
                    resolve_import_aliases_in_expr(if_clause, import_aliases);
                }
            }
        }
        Expr::DictComp(comp) => {
            resolve_import_aliases_in_expr(&mut comp.key, import_aliases);
            resolve_import_aliases_in_expr(&mut comp.value, import_aliases);
            for generator in &mut comp.generators {
                resolve_import_aliases_in_expr(&mut generator.iter, import_aliases);
                for if_clause in &mut generator.ifs {
                    resolve_import_aliases_in_expr(if_clause, import_aliases);
                }
            }
        }
        Expr::Generator(gen_expr) => {
            resolve_import_aliases_in_expr(&mut gen_expr.elt, import_aliases);
            for generator in &mut gen_expr.generators {
                resolve_import_aliases_in_expr(&mut generator.iter, import_aliases);
                for if_clause in &mut generator.ifs {
                    resolve_import_aliases_in_expr(if_clause, import_aliases);
                }
            }
        }
        Expr::BoolOp(bool_op) => {
            for value in &mut bool_op.values {
                resolve_import_aliases_in_expr(value, import_aliases);
            }
        }
        Expr::UnaryOp(unary) => {
            resolve_import_aliases_in_expr(&mut unary.operand, import_aliases);
        }
        Expr::BinOp(bin_op) => {
            resolve_import_aliases_in_expr(&mut bin_op.left, import_aliases);
            resolve_import_aliases_in_expr(&mut bin_op.right, import_aliases);
        }
        Expr::Compare(cmp) => {
            resolve_import_aliases_in_expr(&mut cmp.left, import_aliases);
            for comparator in &mut cmp.comparators {
                resolve_import_aliases_in_expr(comparator, import_aliases);
            }
        }
        Expr::If(if_exp) => {
            resolve_import_aliases_in_expr(&mut if_exp.test, import_aliases);
            resolve_import_aliases_in_expr(&mut if_exp.body, import_aliases);
            resolve_import_aliases_in_expr(&mut if_exp.orelse, import_aliases);
        }
        Expr::Lambda(lambda) => {
            if let Some(ref mut params) = lambda.parameters {
                for arg in &mut params.args {
                    if let Some(ref mut default) = arg.default {
                        resolve_import_aliases_in_expr(default, import_aliases);
                    }
                }
            }
            resolve_import_aliases_in_expr(&mut lambda.body, import_aliases);
        }
        Expr::Await(await_expr) => {
            resolve_import_aliases_in_expr(&mut await_expr.value, import_aliases);
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                resolve_import_aliases_in_expr(value, import_aliases);
            }
        }
        Expr::YieldFrom(yield_from) => {
            resolve_import_aliases_in_expr(&mut yield_from.value, import_aliases);
        }
        Expr::Starred(starred) => {
            resolve_import_aliases_in_expr(&mut starred.value, import_aliases);
        }
        Expr::Named(named) => {
            resolve_import_aliases_in_expr(&mut named.target, import_aliases);
            resolve_import_aliases_in_expr(&mut named.value, import_aliases);
        }
        Expr::Slice(slice) => {
            if let Some(ref mut lower) = slice.lower {
                resolve_import_aliases_in_expr(lower, import_aliases);
            }
            if let Some(ref mut upper) = slice.upper {
                resolve_import_aliases_in_expr(upper, import_aliases);
            }
            if let Some(ref mut step) = slice.step {
                resolve_import_aliases_in_expr(step, import_aliases);
            }
        }
        Expr::FString(fstring) => {
            // Handle f-string interpolations
            for element in fstring.value.elements() {
                if let ruff_python_ast::InterpolatedStringElement::Interpolation(interp) = element {
                    let mut expr_clone = (*interp.expression).clone();
                    resolve_import_aliases_in_expr(&mut expr_clone, import_aliases);
                    // Note: We can't modify the expression in-place here due to the iterator
                    // This would require a more complex transformation
                }
            }
        }
        // Literals don't need alias resolution
        Expr::StringLiteral(_)
        | Expr::BytesLiteral(_)
        | Expr::NumberLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_)
        | Expr::EllipsisLiteral(_)
        | Expr::IpyEscapeCommand(_)
        | Expr::TString(_) => {}
    }
}

/// Rewrite aliases in expression using the bundler's alias mappings
pub fn rewrite_aliases_in_expr(expr: &mut Expr, alias_to_canonical: &FxIndexMap<String, String>) {
    match expr {
        Expr::Name(name_expr) => {
            // Check if this is an aliased import that should be rewritten
            if let Some(canonical) = alias_to_canonical.get(name_expr.id.as_str()) {
                debug!(
                    "Rewriting name '{}' to '{}' (context: {:?})",
                    name_expr.id.as_str(),
                    canonical,
                    name_expr.ctx
                );
                // For Store context (assignments), we only rename simple names
                if matches!(name_expr.ctx, ExprContext::Store) {
                    name_expr.id = canonical.clone().into();
                } else {
                    // For Load context, handle dotted names
                    if canonical.contains('.') {
                        // Convert to attribute access (e.g., pathlib.Path)
                        let parts: Vec<&str> = canonical.split('.').collect();
                        *expr = expressions::dotted_name(&parts, name_expr.ctx);
                    } else {
                        // Simple module name
                        name_expr.id = canonical.clone().into();
                    }
                }
            }
        }
        Expr::Attribute(attr_expr) => {
            // Recursively process the value - this will correctly handle
            // dotted names like "pathlib.Path" by converting them to proper
            // attribute expressions
            rewrite_aliases_in_expr(&mut attr_expr.value, alias_to_canonical);
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

            rewrite_aliases_in_expr(&mut call_expr.func, alias_to_canonical);
            for (i, arg) in call_expr.arguments.args.iter_mut().enumerate() {
                debug!("  Rewriting call arg {i}");
                rewrite_aliases_in_expr(arg, alias_to_canonical);
            }
            for keyword in &mut call_expr.arguments.keywords {
                rewrite_aliases_in_expr(&mut keyword.value, alias_to_canonical);
            }
        }
        // Handle other expression types recursively
        Expr::List(list_expr) => {
            debug!("Rewriting aliases in list expression");
            for elem in &mut list_expr.elts {
                rewrite_aliases_in_expr(elem, alias_to_canonical);
            }
        }
        Expr::Tuple(tuple_expr) => {
            debug!("Rewriting aliases in tuple expression");
            for elem in &mut tuple_expr.elts {
                rewrite_aliases_in_expr(elem, alias_to_canonical);
            }
        }
        Expr::Subscript(subscript_expr) => {
            debug!("Rewriting aliases in subscript expression");
            rewrite_aliases_in_expr(&mut subscript_expr.value, alias_to_canonical);
            rewrite_aliases_in_expr(&mut subscript_expr.slice, alias_to_canonical);
        }
        Expr::BinOp(binop) => {
            rewrite_aliases_in_expr(&mut binop.left, alias_to_canonical);
            rewrite_aliases_in_expr(&mut binop.right, alias_to_canonical);
        }
        Expr::UnaryOp(unaryop) => {
            rewrite_aliases_in_expr(&mut unaryop.operand, alias_to_canonical);
        }
        Expr::Compare(compare) => {
            rewrite_aliases_in_expr(&mut compare.left, alias_to_canonical);
            for comparator in &mut compare.comparators {
                rewrite_aliases_in_expr(comparator, alias_to_canonical);
            }
        }
        Expr::BoolOp(boolop) => {
            for value in &mut boolop.values {
                rewrite_aliases_in_expr(value, alias_to_canonical);
            }
        }
        Expr::Dict(dict) => {
            for item in &mut dict.items {
                if let Some(ref mut key) = item.key {
                    rewrite_aliases_in_expr(key, alias_to_canonical);
                }
                rewrite_aliases_in_expr(&mut item.value, alias_to_canonical);
            }
        }
        Expr::Set(set) => {
            for elem in &mut set.elts {
                rewrite_aliases_in_expr(elem, alias_to_canonical);
            }
        }
        Expr::ListComp(comp) => {
            rewrite_aliases_in_expr(&mut comp.elt, alias_to_canonical);
            for generator in &mut comp.generators {
                rewrite_aliases_in_expr(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::SetComp(comp) => {
            rewrite_aliases_in_expr(&mut comp.elt, alias_to_canonical);
            for generator in &mut comp.generators {
                rewrite_aliases_in_expr(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::DictComp(comp) => {
            rewrite_aliases_in_expr(&mut comp.key, alias_to_canonical);
            rewrite_aliases_in_expr(&mut comp.value, alias_to_canonical);
            for generator in &mut comp.generators {
                rewrite_aliases_in_expr(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::Generator(comp) => {
            rewrite_aliases_in_expr(&mut comp.elt, alias_to_canonical);
            for generator in &mut comp.generators {
                rewrite_aliases_in_expr(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::Lambda(lambda) => {
            // Lambda parameters might have annotations
            if let Some(ref mut params) = lambda.parameters {
                for param in &mut params.posonlyargs {
                    if let Some(ref mut annotation) = param.parameter.annotation {
                        rewrite_aliases_in_expr(annotation, alias_to_canonical);
                    }
                }
                for param in &mut params.args {
                    if let Some(ref mut annotation) = param.parameter.annotation {
                        rewrite_aliases_in_expr(annotation, alias_to_canonical);
                    }
                }
                for param in &mut params.kwonlyargs {
                    if let Some(ref mut annotation) = param.parameter.annotation {
                        rewrite_aliases_in_expr(annotation, alias_to_canonical);
                    }
                }
            }
            // Process the body
            rewrite_aliases_in_expr(&mut lambda.body, alias_to_canonical);
        }
        Expr::If(ifexp) => {
            rewrite_aliases_in_expr(&mut ifexp.test, alias_to_canonical);
            rewrite_aliases_in_expr(&mut ifexp.body, alias_to_canonical);
            rewrite_aliases_in_expr(&mut ifexp.orelse, alias_to_canonical);
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                rewrite_aliases_in_expr(value, alias_to_canonical);
            }
        }
        Expr::YieldFrom(yield_from) => {
            rewrite_aliases_in_expr(&mut yield_from.value, alias_to_canonical);
        }
        Expr::Await(await_expr) => {
            rewrite_aliases_in_expr(&mut await_expr.value, alias_to_canonical);
        }
        Expr::Starred(starred) => {
            rewrite_aliases_in_expr(&mut starred.value, alias_to_canonical);
        }
        Expr::Slice(slice) => {
            if let Some(ref mut lower) = slice.lower {
                rewrite_aliases_in_expr(lower, alias_to_canonical);
            }
            if let Some(ref mut upper) = slice.upper {
                rewrite_aliases_in_expr(upper, alias_to_canonical);
            }
            if let Some(ref mut step) = slice.step {
                rewrite_aliases_in_expr(step, alias_to_canonical);
            }
        }
        Expr::Named(named) => {
            rewrite_aliases_in_expr(&mut named.target, alias_to_canonical);
            rewrite_aliases_in_expr(&mut named.value, alias_to_canonical);
        }
        Expr::FString(fstring) => {
            // Handle f-string interpolations by transforming each expression element
            let mut new_elements = Vec::new();
            let mut any_changed = false;

            for element in fstring.value.elements() {
                match element {
                    ruff_python_ast::InterpolatedStringElement::Literal(lit_elem) => {
                        // Literal elements don't contain expressions, so just clone them
                        new_elements.push(ruff_python_ast::InterpolatedStringElement::Literal(
                            lit_elem.clone(),
                        ));
                    }
                    ruff_python_ast::InterpolatedStringElement::Interpolation(expr_elem) => {
                        // Clone the expression and rewrite aliases in it
                        let mut new_expr = (*expr_elem.expression).clone();
                        let old_expr_debug = format!("{new_expr:?}");
                        rewrite_aliases_in_expr(&mut new_expr, alias_to_canonical);
                        let new_expr_debug = format!("{new_expr:?}");

                        if old_expr_debug != new_expr_debug {
                            any_changed = true;
                        }

                        // Create a new interpolation element with the rewritten expression
                        let new_element = ruff_python_ast::InterpolatedElement {
                            node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                            expression: Box::new(new_expr),
                            debug_text: expr_elem.debug_text.clone(),
                            conversion: expr_elem.conversion,
                            format_spec: expr_elem.format_spec.clone(),
                            range: expr_elem.range,
                        };

                        new_elements.push(
                            ruff_python_ast::InterpolatedStringElement::Interpolation(new_element),
                        );
                    }
                }
            }

            // If any expressions were changed, rebuild the f-string
            if any_changed {
                // Preserve the original flags from the f-string
                let original_flags = if let Some(fstring_part) =
                    fstring.value.iter().find_map(|part| match part {
                        ruff_python_ast::FStringPart::FString(f) => Some(f),
                        _ => None,
                    }) {
                    fstring_part.flags
                } else {
                    ruff_python_ast::FStringFlags::empty()
                };
                let new_fstring = ruff_python_ast::FString {
                    node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                    elements: ruff_python_ast::InterpolatedStringElements::from(new_elements),
                    range: ruff_text_size::TextRange::default(),
                    flags: original_flags, // Preserve the original flags including quote style
                };

                let new_value = ruff_python_ast::FStringValue::single(new_fstring);

                *expr = Expr::FString(ruff_python_ast::ExprFString {
                    node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                    value: new_value,
                    range: fstring.range,
                });
            }
        }
        // For literal expressions and other complex types, no rewriting needed
        _ => {}
    }
}

/// Transform expression for lifted globals
pub(super) fn transform_expr_for_lifted_globals(
    bundler: &Bundler,
    expr: &mut Expr,
    lifted_names: &FxIndexMap<String, String>,
    global_info: &crate::semantic_bundler::ModuleGlobalInfo,
    in_function_with_globals: Option<&FxIndexSet<String>>,
) {
    match expr {
        Expr::Name(name_expr) => {
            // Transform if this is a lifted global and we're in a function that declares it
            // global
            if let Some(function_globals) = in_function_with_globals
                && function_globals.contains(name_expr.id.as_str())
                && let Some(lifted_name) = lifted_names.get(name_expr.id.as_str())
            {
                name_expr.id = lifted_name.clone().into();
            }
        }
        Expr::Attribute(attr_expr) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut attr_expr.value,
                lifted_names,
                global_info,
                in_function_with_globals,
            );
        }
        Expr::Call(call_expr) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut call_expr.func,
                lifted_names,
                global_info,
                in_function_with_globals,
            );
            for arg in &mut call_expr.arguments.args {
                transform_expr_for_lifted_globals(
                    bundler,
                    arg,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            for keyword in &mut call_expr.arguments.keywords {
                transform_expr_for_lifted_globals(
                    bundler,
                    &mut keyword.value,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
        }
        Expr::FString(_) => {
            transform_fstring_for_lifted_globals(
                bundler,
                expr,
                lifted_names,
                global_info,
                in_function_with_globals,
            );
        }
        Expr::BinOp(binop) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut binop.left,
                lifted_names,
                global_info,
                in_function_with_globals,
            );
            transform_expr_for_lifted_globals(
                bundler,
                &mut binop.right,
                lifted_names,
                global_info,
                in_function_with_globals,
            );
        }
        Expr::UnaryOp(unaryop) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut unaryop.operand,
                lifted_names,
                global_info,
                in_function_with_globals,
            );
        }
        Expr::Compare(compare) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut compare.left,
                lifted_names,
                global_info,
                in_function_with_globals,
            );
            for comparator in &mut compare.comparators {
                transform_expr_for_lifted_globals(
                    bundler,
                    comparator,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
        }
        Expr::Subscript(subscript) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut subscript.value,
                lifted_names,
                global_info,
                in_function_with_globals,
            );
            transform_expr_for_lifted_globals(
                bundler,
                &mut subscript.slice,
                lifted_names,
                global_info,
                in_function_with_globals,
            );
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                transform_expr_for_lifted_globals(
                    bundler,
                    elem,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
        }
        Expr::Tuple(tuple_expr) => {
            for elem in &mut tuple_expr.elts {
                transform_expr_for_lifted_globals(
                    bundler,
                    elem,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
        }
        Expr::Dict(dict_expr) => {
            for item in &mut dict_expr.items {
                if let Some(key) = &mut item.key {
                    transform_expr_for_lifted_globals(
                        bundler,
                        key,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
                transform_expr_for_lifted_globals(
                    bundler,
                    &mut item.value,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
        }
        _ => {
            // Other expressions handled as needed
        }
    }
}

/// Transform f-string expressions for lifted globals
pub(super) fn transform_fstring_for_lifted_globals(
    bundler: &Bundler,
    expr: &mut Expr,
    lifted_names: &FxIndexMap<String, String>,
    global_info: &crate::semantic_bundler::ModuleGlobalInfo,
    in_function_with_globals: Option<&FxIndexSet<String>>,
) {
    if let Expr::FString(fstring) = expr {
        let fstring_range = fstring.range;
        let mut transformed_elements = Vec::new();
        let mut any_transformed = false;

        for element in fstring.value.elements() {
            match element {
                ruff_python_ast::InterpolatedStringElement::Literal(lit_elem) => {
                    // Literal elements stay the same
                    transformed_elements.push(ruff_python_ast::InterpolatedStringElement::Literal(
                        lit_elem.clone(),
                    ));
                }
                ruff_python_ast::InterpolatedStringElement::Interpolation(expr_elem) => {
                    let (new_element, was_transformed) = transform_fstring_expression(
                        bundler,
                        expr_elem,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                    transformed_elements.push(
                        ruff_python_ast::InterpolatedStringElement::Interpolation(new_element),
                    );
                    if was_transformed {
                        any_transformed = true;
                    }
                }
            }
        }

        // If any expressions were transformed, we need to rebuild the f-string
        if any_transformed {
            // Preserve the original flags from the f-string
            let original_flags = crate::ast_builder::expressions::get_fstring_flags(&fstring.value);
            // Create a new FString with our transformed elements
            let new_fstring = ruff_python_ast::FString {
                node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                elements: ruff_python_ast::InterpolatedStringElements::from(transformed_elements),
                range: ruff_text_size::TextRange::default(),
                flags: original_flags, // Preserve the original flags including quote style
            };

            // Create a new FStringValue containing our FString
            let new_value = ruff_python_ast::FStringValue::single(new_fstring);

            // Replace the entire expression with the new f-string
            *expr = Expr::FString(ruff_python_ast::ExprFString {
                node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                value: new_value,
                range: fstring_range,
            });

            log::debug!("Transformed f-string expressions for lifted globals");
        }
    }
}

/// Transform a single f-string expression element
pub(super) fn transform_fstring_expression(
    bundler: &Bundler,
    expr_elem: &ruff_python_ast::InterpolatedElement,
    lifted_names: &FxIndexMap<String, String>,
    global_info: &crate::semantic_bundler::ModuleGlobalInfo,
    in_function_with_globals: Option<&FxIndexSet<String>>,
) -> (ruff_python_ast::InterpolatedElement, bool) {
    // Clone and transform the expression
    let mut new_expr = (*expr_elem.expression).clone();
    let old_expr_str = format!("{new_expr:?}");

    transform_expr_for_lifted_globals(
        bundler,
        &mut new_expr,
        lifted_names,
        global_info,
        in_function_with_globals,
    );

    let new_expr_str = format!("{new_expr:?}");
    let was_transformed = old_expr_str != new_expr_str;

    // Create a new expression element with the transformed expression
    let new_element = ruff_python_ast::InterpolatedElement {
        node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
        expression: Box::new(new_expr),
        debug_text: expr_elem.debug_text.clone(),
        conversion: expr_elem.conversion,
        format_spec: expr_elem.format_spec.clone(),
        range: expr_elem.range,
    };

    (new_element, was_transformed)
}

/// Recursively rewrite aliases in statements
pub fn rewrite_aliases_in_stmt(stmt: &mut Stmt, alias_to_canonical: &FxIndexMap<String, String>) {
    match stmt {
        Stmt::Expr(expr_stmt) => {
            rewrite_aliases_in_expr(&mut expr_stmt.value, alias_to_canonical);
        }
        Stmt::Assign(assign) => {
            rewrite_aliases_in_expr(&mut assign.value, alias_to_canonical);
            for target in &mut assign.targets {
                rewrite_aliases_in_expr(target, alias_to_canonical);
            }
        }
        Stmt::Return(return_stmt) => {
            debug!("Rewriting aliases in return statement");
            if let Some(ref mut value) = return_stmt.value {
                rewrite_aliases_in_expr(value, alias_to_canonical);
            }
        }
        Stmt::FunctionDef(func_def) => {
            rewrite_aliases_in_function(func_def, alias_to_canonical);
        }
        Stmt::ClassDef(class_def) => {
            rewrite_aliases_in_class(class_def, alias_to_canonical);
        }
        Stmt::AnnAssign(ann_assign) => {
            // Rewrite the annotation
            rewrite_aliases_in_expr(&mut ann_assign.annotation, alias_to_canonical);
            // Rewrite the target
            rewrite_aliases_in_expr(&mut ann_assign.target, alias_to_canonical);
            // Rewrite the value if present
            if let Some(ref mut value) = ann_assign.value {
                rewrite_aliases_in_expr(value, alias_to_canonical);
            }
        }
        Stmt::If(if_stmt) => {
            rewrite_aliases_in_expr(&mut if_stmt.test, alias_to_canonical);
            for stmt in &mut if_stmt.body {
                rewrite_aliases_in_stmt(stmt, alias_to_canonical);
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(ref mut condition) = clause.test {
                    rewrite_aliases_in_expr(condition, alias_to_canonical);
                }
                for stmt in &mut clause.body {
                    rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
        }
        Stmt::While(while_stmt) => {
            rewrite_aliases_in_expr(&mut while_stmt.test, alias_to_canonical);
            for stmt in &mut while_stmt.body {
                rewrite_aliases_in_stmt(stmt, alias_to_canonical);
            }
            for stmt in &mut while_stmt.orelse {
                rewrite_aliases_in_stmt(stmt, alias_to_canonical);
            }
        }
        Stmt::For(for_stmt) => {
            rewrite_aliases_in_expr(&mut for_stmt.iter, alias_to_canonical);
            for stmt in &mut for_stmt.body {
                rewrite_aliases_in_stmt(stmt, alias_to_canonical);
            }
            for stmt in &mut for_stmt.orelse {
                rewrite_aliases_in_stmt(stmt, alias_to_canonical);
            }
        }
        Stmt::With(with_stmt) => {
            for item in &mut with_stmt.items {
                rewrite_aliases_in_expr(&mut item.context_expr, alias_to_canonical);
            }
            for stmt in &mut with_stmt.body {
                rewrite_aliases_in_stmt(stmt, alias_to_canonical);
            }
        }
        Stmt::Try(try_stmt) => {
            for stmt in &mut try_stmt.body {
                rewrite_aliases_in_stmt(stmt, alias_to_canonical);
            }
            for handler in &mut try_stmt.handlers {
                rewrite_aliases_in_except_handler(handler, alias_to_canonical);
            }
            for stmt in &mut try_stmt.orelse {
                rewrite_aliases_in_stmt(stmt, alias_to_canonical);
            }
            for stmt in &mut try_stmt.finalbody {
                rewrite_aliases_in_stmt(stmt, alias_to_canonical);
            }
        }
        Stmt::AugAssign(aug_assign) => {
            rewrite_aliases_in_expr(&mut aug_assign.target, alias_to_canonical);
            rewrite_aliases_in_expr(&mut aug_assign.value, alias_to_canonical);
        }
        Stmt::Raise(raise_stmt) => {
            if let Some(ref mut exc) = raise_stmt.exc {
                rewrite_aliases_in_expr(exc, alias_to_canonical);
            }
            if let Some(ref mut cause) = raise_stmt.cause {
                rewrite_aliases_in_expr(cause, alias_to_canonical);
            }
        }
        Stmt::Assert(assert_stmt) => {
            rewrite_aliases_in_expr(&mut assert_stmt.test, alias_to_canonical);
            if let Some(ref mut msg) = assert_stmt.msg {
                rewrite_aliases_in_expr(msg, alias_to_canonical);
            }
        }
        Stmt::Delete(delete_stmt) => {
            for target in &mut delete_stmt.targets {
                rewrite_aliases_in_expr(target, alias_to_canonical);
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

/// Rewrite aliases in exception handlers
fn rewrite_aliases_in_except_handler(
    handler: &mut ExceptHandler,
    alias_to_canonical: &FxIndexMap<String, String>,
) {
    match handler {
        ExceptHandler::ExceptHandler(except_handler) => {
            if let Some(ref mut type_) = except_handler.type_ {
                rewrite_aliases_in_expr(type_, alias_to_canonical);
            }
            for stmt in &mut except_handler.body {
                rewrite_aliases_in_stmt(stmt, alias_to_canonical);
            }
        }
    }
}

/// Rewrite aliases in function definitions
fn rewrite_aliases_in_function(
    func_def: &mut StmtFunctionDef,
    alias_to_canonical: &FxIndexMap<String, String>,
) {
    debug!("Rewriting aliases in function: {}", func_def.name.as_str());

    // Rewrite parameter annotations
    for param in &mut func_def.parameters.posonlyargs {
        if let Some(ref mut annotation) = param.parameter.annotation {
            rewrite_aliases_in_expr(annotation, alias_to_canonical);
        }
    }
    for param in &mut func_def.parameters.args {
        if let Some(ref mut annotation) = param.parameter.annotation {
            rewrite_aliases_in_expr(annotation, alias_to_canonical);
        }
    }
    for param in &mut func_def.parameters.kwonlyargs {
        if let Some(ref mut annotation) = param.parameter.annotation {
            rewrite_aliases_in_expr(annotation, alias_to_canonical);
        }
    }
    if let Some(ref mut vararg) = func_def.parameters.vararg
        && let Some(ref mut annotation) = vararg.annotation
    {
        rewrite_aliases_in_expr(annotation, alias_to_canonical);
    }
    if let Some(ref mut kwarg) = func_def.parameters.kwarg
        && let Some(ref mut annotation) = kwarg.annotation
    {
        rewrite_aliases_in_expr(annotation, alias_to_canonical);
    }

    // Rewrite return type annotation
    if let Some(ref mut returns) = func_def.returns {
        rewrite_aliases_in_expr(returns, alias_to_canonical);
    }

    // First handle global statements specially
    rewrite_global_statements_in_function(func_def, alias_to_canonical);

    // Hoist and deduplicate any global statements to the start of the function body
    // (after a possible docstring). This prevents Python errors where a name is
    // used before its corresponding `global` declaration within the same function.
    hoist_and_dedup_global_statements(func_def);

    // Then rewrite all non-global statements in the function body
    for (idx, stmt) in func_def.body.iter_mut().enumerate() {
        // Skip global statements as they've already been processed
        if !matches!(stmt, Stmt::Global(_)) {
            debug!(
                "  Rewriting aliases in function body statement {}: {:?}",
                idx,
                std::mem::discriminant(stmt)
            );
            rewrite_aliases_in_stmt(stmt, alias_to_canonical);
        }
    }
}

/// Rewrite only global statements in function, not other references
fn rewrite_global_statements_in_function(
    func_def: &mut StmtFunctionDef,
    alias_to_canonical: &FxIndexMap<String, String>,
) {
    for stmt in &mut func_def.body {
        rewrite_global_statements_only(stmt, alias_to_canonical);
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
                rewrite_global_statements_only(stmt, alias_to_canonical);
            }
            for clause in &mut if_stmt.elif_else_clauses {
                for stmt in &mut clause.body {
                    rewrite_global_statements_only(stmt, alias_to_canonical);
                }
            }
        }
        Stmt::For(for_stmt) => {
            for stmt in &mut for_stmt.body {
                rewrite_global_statements_only(stmt, alias_to_canonical);
            }
            for stmt in &mut for_stmt.orelse {
                rewrite_global_statements_only(stmt, alias_to_canonical);
            }
        }
        Stmt::While(while_stmt) => {
            for stmt in &mut while_stmt.body {
                rewrite_global_statements_only(stmt, alias_to_canonical);
            }
            for stmt in &mut while_stmt.orelse {
                rewrite_global_statements_only(stmt, alias_to_canonical);
            }
        }
        Stmt::With(with_stmt) => {
            for stmt in &mut with_stmt.body {
                rewrite_global_statements_only(stmt, alias_to_canonical);
            }
        }
        Stmt::Try(try_stmt) => {
            for stmt in &mut try_stmt.body {
                rewrite_global_statements_only(stmt, alias_to_canonical);
            }
            for handler in &mut try_stmt.handlers {
                match handler {
                    ExceptHandler::ExceptHandler(except_handler) => {
                        for stmt in &mut except_handler.body {
                            rewrite_global_statements_only(stmt, alias_to_canonical);
                        }
                    }
                }
            }
            for stmt in &mut try_stmt.orelse {
                rewrite_global_statements_only(stmt, alias_to_canonical);
            }
            for stmt in &mut try_stmt.finalbody {
                rewrite_global_statements_only(stmt, alias_to_canonical);
            }
        }
        _ => {}
    }
}

/// Move all `global` statements in a function to the start of the function body
/// (after a leading docstring, if present) and deduplicate their names.
fn hoist_and_dedup_global_statements(func_def: &mut StmtFunctionDef) {
    use ruff_python_ast::helpers::is_docstring_stmt;

    // Collect all global names in order of first appearance
    let mut names: FxIndexSet<String> = FxIndexSet::default();
    let mut has_global = false;

    for stmt in &func_def.body {
        if let Stmt::Global(g) = stmt {
            has_global = true;
            for ident in &g.names {
                names.insert(ident.as_str().to_string());
            }
        }
    }

    if !has_global {
        return;
    }

    debug!("Hoisting {} global name(s) to function start", names.len());

    // Rebuild body without any existing global statements
    let mut new_body: Vec<Stmt> = Vec::with_capacity(func_def.body.len());
    for stmt in func_def.body.drain(..) {
        if !matches!(stmt, Stmt::Global(_)) {
            new_body.push(stmt);
        }
    }

    // Determine insertion index: after a leading docstring statement, if any
    let insert_at = usize::from(new_body.first().is_some_and(is_docstring_stmt));

    // Build a single combined global statement with deduplicated names
    let combined_global = Stmt::Global(ruff_python_ast::StmtGlobal {
        names: names
            .into_iter()
            .map(|s| Identifier::new(s, TextRange::default()))
            .collect(),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    });

    // Insert the combined global at the computed position
    new_body.insert(insert_at, combined_global);

    // Replace function body
    func_def.body = new_body;
}

/// Rewrite aliases in class definitions
fn rewrite_aliases_in_class(
    class_def: &mut StmtClassDef,
    alias_to_canonical: &FxIndexMap<String, String>,
) {
    // Rewrite base classes and keyword arguments
    if let Some(arguments) = &mut class_def.arguments {
        for base in &mut arguments.args {
            rewrite_aliases_in_expr(base, alias_to_canonical);
        }
        // Also rewrite keyword arguments (e.g., metaclass=SomeMetaclass)
        for keyword in &mut arguments.keywords {
            rewrite_aliases_in_expr(&mut keyword.value, alias_to_canonical);
        }
    }

    // Rewrite class body
    for stmt in &mut class_def.body {
        rewrite_aliases_in_stmt(stmt, alias_to_canonical);
    }
}

/// Extract target name from a simple assignment
pub fn extract_simple_assign_target(assign: &StmtAssign) -> Option<String> {
    if assign.targets.len() == 1
        && let Expr::Name(name) = &assign.targets[0]
    {
        return Some(name.id.to_string());
    }
    None
}

/// Check if an assignment is self-referential (e.g., x = x)
pub fn is_self_referential_assignment(assign: &StmtAssign, python_version: u8) -> bool {
    // Check if this is a simple assignment with a single target and value
    if assign.targets.len() == 1
        && let (Expr::Name(target), Expr::Name(value)) = (&assign.targets[0], assign.value.as_ref())
    {
        // Check if target and value have the same name
        if target.id == value.id {
            // Special case: Built-in types like `bytes = bytes`, `str = str` are NOT
            // self-referential They're re-exporting Python's built-in types
            // through the module's namespace
            let name = target.id.as_str();
            if ruff_python_stdlib::builtins::is_python_builtin(name, python_version, false) {
                log::debug!(
                    "Assignment '{}' = '{}' is a built-in type re-export, not self-referential",
                    target.id,
                    value.id
                );
                return false;
            }

            log::debug!(
                "Found self-referential assignment: {} = {}",
                target.id,
                value.id
            );
            return true;
        }
    }
    false
}
