//! Expression handling utilities for code generation
//!
//! This module contains functions for creating, analyzing, and transforming
//! `rustpython_parser::ast::Expr` nodes during the bundling process.

use ruff_python_ast::{Expr, ExprAttribute, ExprContext, Stmt};

use super::bundler::HybridStaticBundler;
use crate::{
    ast_builder::{expressions, statements},
    types::{FxIndexMap, FxIndexSet},
};

/// Check if an expression uses importlib
pub(super) fn expr_uses_importlib(expr: &Expr) -> bool {
    match expr {
        Expr::Name(name) => name.id.as_str() == "importlib",
        Expr::Attribute(attr) => expr_uses_importlib(&attr.value),
        Expr::Call(call) => {
            expr_uses_importlib(&call.func)
                || call.arguments.args.iter().any(expr_uses_importlib)
                || call
                    .arguments
                    .keywords
                    .iter()
                    .any(|kw| expr_uses_importlib(&kw.value))
        }
        Expr::Subscript(sub) => expr_uses_importlib(&sub.value) || expr_uses_importlib(&sub.slice),
        Expr::Tuple(tuple) => tuple.elts.iter().any(expr_uses_importlib),
        Expr::List(list) => list.elts.iter().any(expr_uses_importlib),
        Expr::Set(set) => set.elts.iter().any(expr_uses_importlib),
        Expr::Dict(dict) => dict.items.iter().any(|item| {
            item.key.as_ref().is_some_and(expr_uses_importlib) || expr_uses_importlib(&item.value)
        }),
        Expr::ListComp(comp) => {
            expr_uses_importlib(&comp.elt)
                || comp.generators.iter().any(|generator| {
                    expr_uses_importlib(&generator.iter)
                        || generator.ifs.iter().any(expr_uses_importlib)
                })
        }
        Expr::SetComp(comp) => {
            expr_uses_importlib(&comp.elt)
                || comp.generators.iter().any(|generator| {
                    expr_uses_importlib(&generator.iter)
                        || generator.ifs.iter().any(expr_uses_importlib)
                })
        }
        Expr::DictComp(comp) => {
            expr_uses_importlib(&comp.key)
                || expr_uses_importlib(&comp.value)
                || comp.generators.iter().any(|generator| {
                    expr_uses_importlib(&generator.iter)
                        || generator.ifs.iter().any(expr_uses_importlib)
                })
        }
        Expr::Generator(generator_exp) => {
            expr_uses_importlib(&generator_exp.elt)
                || generator_exp
                    .generators
                    .iter()
                    .any(|g| expr_uses_importlib(&g.iter) || g.ifs.iter().any(expr_uses_importlib))
        }
        Expr::BoolOp(bool_op) => bool_op.values.iter().any(expr_uses_importlib),
        Expr::UnaryOp(unary) => expr_uses_importlib(&unary.operand),
        Expr::BinOp(bin_op) => {
            expr_uses_importlib(&bin_op.left) || expr_uses_importlib(&bin_op.right)
        }
        Expr::Compare(cmp) => {
            expr_uses_importlib(&cmp.left) || cmp.comparators.iter().any(expr_uses_importlib)
        }
        Expr::If(if_exp) => {
            expr_uses_importlib(&if_exp.test)
                || expr_uses_importlib(&if_exp.body)
                || expr_uses_importlib(&if_exp.orelse)
        }
        Expr::Lambda(lambda) => {
            // Check default parameter values
            lambda.parameters.as_ref().is_some_and(|params| {
                params
                    .args
                    .iter()
                    .any(|arg| arg.default.as_ref().is_some_and(|d| expr_uses_importlib(d)))
            }) || expr_uses_importlib(&lambda.body)
        }
        Expr::Await(await_expr) => expr_uses_importlib(&await_expr.value),
        Expr::Yield(yield_expr) => yield_expr
            .value
            .as_ref()
            .is_some_and(|v| expr_uses_importlib(v)),
        Expr::YieldFrom(yield_from) => expr_uses_importlib(&yield_from.value),
        Expr::Starred(starred) => expr_uses_importlib(&starred.value),
        Expr::Named(named) => {
            expr_uses_importlib(&named.target) || expr_uses_importlib(&named.value)
        }
        Expr::Slice(slice) => {
            slice.lower.as_ref().is_some_and(|l| expr_uses_importlib(l))
                || slice.upper.as_ref().is_some_and(|u| expr_uses_importlib(u))
                || slice.step.as_ref().is_some_and(|s| expr_uses_importlib(s))
        }
        // Literals don't use importlib
        Expr::StringLiteral(_)
        | Expr::BytesLiteral(_)
        | Expr::NumberLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_)
        | Expr::EllipsisLiteral(_) => false,
        // Check if any interpolated expressions in the f-string use importlib
        Expr::FString(fstring) => fstring.value.elements().any(|element| {
            if let ruff_python_ast::InterpolatedStringElement::Interpolation(expr_elem) = element {
                expr_uses_importlib(&expr_elem.expression)
            } else {
                false
            }
        }),
        // Match expressions not available in this ruff version
        // Type expressions typically don't use importlib directly
        Expr::IpyEscapeCommand(_) => false, // IPython specific
        Expr::TString(_) => false,          // Template strings
    }
}

/// Extract attribute path from expression (e.g., "foo.bar.baz" from foo.bar.baz)
pub(super) fn extract_attribute_path(
    _bundler: &HybridStaticBundler,
    attr: &ExprAttribute,
) -> String {
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

/// Extract string list from expression (used for parsing __all__ declarations)
pub(super) fn extract_string_list_from_expr(
    _bundler: &HybridStaticBundler,
    expr: &Expr,
) -> Option<Vec<String>> {
    match expr {
        Expr::List(list_expr) => {
            let mut exports = Vec::new();
            for element in &list_expr.elts {
                if let Expr::StringLiteral(string_lit) = element {
                    let string_value = string_lit.value.to_str();
                    exports.push(string_value.to_string());
                }
            }
            Some(exports)
        }
        Expr::Tuple(tuple_expr) => {
            let mut exports = Vec::new();
            for element in &tuple_expr.elts {
                if let Expr::StringLiteral(string_lit) = element {
                    let string_value = string_lit.value.to_str();
                    exports.push(string_value.to_string());
                }
            }
            Some(exports)
        }
        _ => None, // Other expressions like computed lists are not supported
    }
}

/// Convert expression to dotted name string
pub(super) fn expr_to_dotted_name(expr: &Expr) -> String {
    match expr {
        Expr::Name(name) => name.id.to_string(),
        Expr::Attribute(attr) => {
            let value_name = expr_to_dotted_name(&attr.value);
            format!("{}.{}", value_name, attr.attr)
        }
        _ => {
            // For non-name expressions, return a placeholder
            "<complex_expr>".to_string()
        }
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
pub(super) fn rewrite_aliases_in_expr(
    _bundler: &HybridStaticBundler,
    expr: &mut Expr,
    alias_to_canonical: &FxIndexMap<String, String>,
) {
    rewrite_aliases_in_expr_impl(expr, alias_to_canonical);
}

/// Implementation of alias rewriting in expressions
pub(super) fn rewrite_aliases_in_expr_impl(
    expr: &mut Expr,
    alias_to_canonical: &FxIndexMap<String, String>,
) {
    match expr {
        Expr::Name(name_expr) => {
            if let Some(canonical_name) = alias_to_canonical.get(name_expr.id.as_str()) {
                name_expr.id = canonical_name.clone().into();
            }
        }
        Expr::Attribute(attr_expr) => {
            rewrite_aliases_in_expr_impl(&mut attr_expr.value, alias_to_canonical);
        }
        Expr::Call(call_expr) => {
            rewrite_aliases_in_expr_impl(&mut call_expr.func, alias_to_canonical);
            for arg in &mut call_expr.arguments.args {
                rewrite_aliases_in_expr_impl(arg, alias_to_canonical);
            }
            for keyword in &mut call_expr.arguments.keywords {
                rewrite_aliases_in_expr_impl(&mut keyword.value, alias_to_canonical);
            }
        }
        Expr::Subscript(subscript_expr) => {
            rewrite_aliases_in_expr_impl(&mut subscript_expr.value, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut subscript_expr.slice, alias_to_canonical);
        }
        Expr::List(list_expr) => {
            for elt in &mut list_expr.elts {
                rewrite_aliases_in_expr_impl(elt, alias_to_canonical);
            }
        }
        Expr::Tuple(tuple_expr) => {
            for elt in &mut tuple_expr.elts {
                rewrite_aliases_in_expr_impl(elt, alias_to_canonical);
            }
        }
        Expr::Set(set_expr) => {
            for elt in &mut set_expr.elts {
                rewrite_aliases_in_expr_impl(elt, alias_to_canonical);
            }
        }
        Expr::Dict(dict_expr) => {
            for item in &mut dict_expr.items {
                if let Some(ref mut key) = item.key {
                    rewrite_aliases_in_expr_impl(key, alias_to_canonical);
                }
                rewrite_aliases_in_expr_impl(&mut item.value, alias_to_canonical);
            }
        }
        Expr::BinOp(binop_expr) => {
            rewrite_aliases_in_expr_impl(&mut binop_expr.left, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut binop_expr.right, alias_to_canonical);
        }
        Expr::UnaryOp(unary_expr) => {
            rewrite_aliases_in_expr_impl(&mut unary_expr.operand, alias_to_canonical);
        }
        Expr::BoolOp(bool_expr) => {
            for value in &mut bool_expr.values {
                rewrite_aliases_in_expr_impl(value, alias_to_canonical);
            }
        }
        Expr::Compare(compare_expr) => {
            rewrite_aliases_in_expr_impl(&mut compare_expr.left, alias_to_canonical);
            for comparator in &mut compare_expr.comparators {
                rewrite_aliases_in_expr_impl(comparator, alias_to_canonical);
            }
        }
        Expr::If(if_expr) => {
            rewrite_aliases_in_expr_impl(&mut if_expr.test, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut if_expr.body, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut if_expr.orelse, alias_to_canonical);
        }
        Expr::Lambda(lambda_expr) => {
            if let Some(ref mut params) = lambda_expr.parameters {
                for arg in &mut params.args {
                    if let Some(ref mut default) = arg.default {
                        rewrite_aliases_in_expr_impl(default, alias_to_canonical);
                    }
                }
            }
            rewrite_aliases_in_expr_impl(&mut lambda_expr.body, alias_to_canonical);
        }
        Expr::ListComp(comp_expr) => {
            rewrite_aliases_in_expr_impl(&mut comp_expr.elt, alias_to_canonical);
            for generator in &mut comp_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::SetComp(comp_expr) => {
            rewrite_aliases_in_expr_impl(&mut comp_expr.elt, alias_to_canonical);
            for generator in &mut comp_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::DictComp(comp_expr) => {
            rewrite_aliases_in_expr_impl(&mut comp_expr.key, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut comp_expr.value, alias_to_canonical);
            for generator in &mut comp_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::Generator(gen_expr) => {
            rewrite_aliases_in_expr_impl(&mut gen_expr.elt, alias_to_canonical);
            for generator in &mut gen_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::Starred(starred_expr) => {
            rewrite_aliases_in_expr_impl(&mut starred_expr.value, alias_to_canonical);
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                rewrite_aliases_in_expr_impl(value, alias_to_canonical);
            }
        }
        Expr::YieldFrom(yield_expr) => {
            rewrite_aliases_in_expr_impl(&mut yield_expr.value, alias_to_canonical);
        }
        Expr::Await(await_expr) => {
            rewrite_aliases_in_expr_impl(&mut await_expr.value, alias_to_canonical);
        }
        Expr::Slice(slice_expr) => {
            if let Some(ref mut lower) = slice_expr.lower {
                rewrite_aliases_in_expr_impl(lower, alias_to_canonical);
            }
            if let Some(ref mut upper) = slice_expr.upper {
                rewrite_aliases_in_expr_impl(upper, alias_to_canonical);
            }
            if let Some(ref mut step) = slice_expr.step {
                rewrite_aliases_in_expr_impl(step, alias_to_canonical);
            }
        }
        Expr::Named(named_expr) => {
            rewrite_aliases_in_expr_impl(&mut named_expr.target, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut named_expr.value, alias_to_canonical);
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
                        rewrite_aliases_in_expr_impl(&mut new_expr, alias_to_canonical);
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
    bundler: &HybridStaticBundler,
    expr: &mut Expr,
    lifted_names: &FxIndexMap<String, String>,
    _global_info: &crate::semantic_bundler::ModuleGlobalInfo,
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
                _global_info,
                in_function_with_globals,
            );
        }
        Expr::Call(call_expr) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut call_expr.func,
                lifted_names,
                _global_info,
                in_function_with_globals,
            );
            for arg in &mut call_expr.arguments.args {
                transform_expr_for_lifted_globals(
                    bundler,
                    arg,
                    lifted_names,
                    _global_info,
                    in_function_with_globals,
                );
            }
            for keyword in &mut call_expr.arguments.keywords {
                transform_expr_for_lifted_globals(
                    bundler,
                    &mut keyword.value,
                    lifted_names,
                    _global_info,
                    in_function_with_globals,
                );
            }
        }
        Expr::FString(_) => {
            transform_fstring_for_lifted_globals(
                bundler,
                expr,
                lifted_names,
                _global_info,
                in_function_with_globals,
            );
        }
        Expr::BinOp(binop) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut binop.left,
                lifted_names,
                _global_info,
                in_function_with_globals,
            );
            transform_expr_for_lifted_globals(
                bundler,
                &mut binop.right,
                lifted_names,
                _global_info,
                in_function_with_globals,
            );
        }
        Expr::UnaryOp(unaryop) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut unaryop.operand,
                lifted_names,
                _global_info,
                in_function_with_globals,
            );
        }
        Expr::Compare(compare) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut compare.left,
                lifted_names,
                _global_info,
                in_function_with_globals,
            );
            for comparator in &mut compare.comparators {
                transform_expr_for_lifted_globals(
                    bundler,
                    comparator,
                    lifted_names,
                    _global_info,
                    in_function_with_globals,
                );
            }
        }
        Expr::Subscript(subscript) => {
            transform_expr_for_lifted_globals(
                bundler,
                &mut subscript.value,
                lifted_names,
                _global_info,
                in_function_with_globals,
            );
            transform_expr_for_lifted_globals(
                bundler,
                &mut subscript.slice,
                lifted_names,
                _global_info,
                in_function_with_globals,
            );
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                transform_expr_for_lifted_globals(
                    bundler,
                    elem,
                    lifted_names,
                    _global_info,
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
                    _global_info,
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
                        _global_info,
                        in_function_with_globals,
                    );
                }
                transform_expr_for_lifted_globals(
                    bundler,
                    &mut item.value,
                    lifted_names,
                    _global_info,
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
    bundler: &HybridStaticBundler,
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
    bundler: &HybridStaticBundler,
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

/// Create namespace attribute assignment
pub(super) fn create_namespace_attribute(
    bundler: &mut HybridStaticBundler,
    parent: &str,
    child: &str,
) -> Stmt {
    // Create: parent.child = types.SimpleNamespace()
    let mut stmt = statements::assign(
        vec![expressions::attribute(
            expressions::name(parent, ExprContext::Load),
            child,
            ExprContext::Store,
        )],
        expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
    );

    // Set the node index for transformation tracking
    if let Stmt::Assign(assign) = &mut stmt {
        assign.node_index = bundler
            .transformation_context
            .create_new_node(format!("Create namespace attribute {parent}.{child}"));
    }

    stmt
}

/// Create dotted attribute assignment
pub(super) fn create_dotted_attribute_assignment(
    bundler: &mut HybridStaticBundler,
    dotted_name: &str,
    value_expr: Expr,
) -> Result<Stmt, String> {
    let parts: Vec<&str> = dotted_name.split('.').collect();
    if parts.is_empty() {
        return Err("Empty dotted name".to_string());
    }

    let target_expr = if parts.len() == 1 {
        expressions::name(parts[0], ExprContext::Store)
    } else {
        let mut expr = expressions::name(parts[0], ExprContext::Load);
        for part in &parts[1..parts.len() - 1] {
            expr = expressions::attribute(expr, part, ExprContext::Load);
        }
        expressions::attribute(expr, parts[parts.len() - 1], ExprContext::Store)
    };

    let mut stmt = statements::assign(vec![target_expr], value_expr);

    // Set the node index for transformation tracking
    if let Stmt::Assign(assign) = &mut stmt {
        assign.node_index = bundler
            .transformation_context
            .create_new_node(format!("create_dotted_attribute_assignment({dotted_name})"));
    }

    Ok(stmt)
}
