//! Shared utilities for visitor implementations

use ruff_python_ast::{Expr, ExprList, ExprName, ExprStringLiteral, ExprTuple};

/// Result of extracting exports from an expression
#[derive(Debug)]
pub(crate) struct ExtractedExports {
    /// The list of exported names if successfully extracted
    pub names: Option<Vec<String>>,
    /// Whether the expression contains dynamic elements
    pub is_dynamic: bool,
}

/// Extract a list of string literals from a List or Tuple expression
/// commonly used for parsing __all__ declarations
///
/// Returns:
/// - `ExtractedExports` with names if all elements are string literals
/// - `ExtractedExports` with `is_dynamic=true` if any element is not a string literal
pub(crate) fn extract_string_list_from_expr(expr: &Expr) -> ExtractedExports {
    match expr {
        Expr::List(ExprList { elts, .. }) | Expr::Tuple(ExprTuple { elts, .. }) => {
            extract_strings_from_elements(elts)
        }
        _ => ExtractedExports {
            names: None,
            is_dynamic: true,
        },
    }
}

/// Extract strings from a slice of expressions
fn extract_strings_from_elements(elts: &[Expr]) -> ExtractedExports {
    let maybe_names: Option<Vec<String>> = elts
        .iter()
        .map(|elt| {
            if let Expr::StringLiteral(ExprStringLiteral { value, .. }) = elt {
                Some(value.to_str().to_owned())
            } else {
                None
            }
        })
        .collect();

    maybe_names.map_or(
        ExtractedExports {
            names: None,
            is_dynamic: true,
        },
        |names| ExtractedExports {
            names: Some(names),
            is_dynamic: false,
        },
    )
}

/// Collect all variable names from an assignment target expression.
///
/// This function handles various assignment patterns including:
/// - Simple names: `x = ...`
/// - Tuple unpacking: `a, b, c = ...`
/// - List unpacking: `[a, b, c] = ...`
/// - Nested unpacking: `(a, (b, c)) = ...`
/// - Starred expressions: `a, *rest = ...`
///
/// Returns a vector of unique variable names found in the target.
/// The names are sorted and deduplicated.
pub(crate) fn collect_names_from_assignment_target(expr: &Expr) -> Vec<&str> {
    let mut names = Vec::new();
    collect_names_recursive(expr, &mut names);
    names.sort_unstable();
    names.dedup();
    names
}

/// Recursively collect variable names from nested assignment targets
fn collect_names_recursive<'a>(expr: &'a Expr, out: &mut Vec<&'a str>) {
    match expr {
        Expr::Name(ExprName { id, .. }) => {
            out.push(id.as_str());
        }
        Expr::Tuple(tuple) => {
            for elem in &tuple.elts {
                collect_names_recursive(elem, out);
            }
        }
        Expr::List(list) => {
            for elem in &list.elts {
                collect_names_recursive(elem, out);
            }
        }
        Expr::Starred(starred) => {
            // Handle starred expressions like *rest in (a, *rest) = ...
            collect_names_recursive(&starred.value, out);
        }
        _ => {
            // Other expression types (like Attribute or Subscript) don't bind new names
            // in assignment contexts, they modify existing objects
        }
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_extract_string_list_from_list() {
        let code = r#"["foo", "bar", "baz"]"#;
        let parsed = parse_module(code).expect("Failed to parse");
        let module = parsed.into_syntax();

        if let Some(ruff_python_ast::Stmt::Expr(expr_stmt)) = module.body.first() {
            let result = extract_string_list_from_expr(&expr_stmt.value);
            assert!(!result.is_dynamic);
            assert_eq!(
                result.names,
                Some(vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()])
            );
        }
    }

    #[test]
    fn test_extract_string_list_with_non_literal() {
        let code = r#"["foo", some_var, "baz"]"#;
        let parsed = parse_module(code).expect("Failed to parse");
        let module = parsed.into_syntax();

        if let Some(ruff_python_ast::Stmt::Expr(expr_stmt)) = module.body.first() {
            let result = extract_string_list_from_expr(&expr_stmt.value);
            assert!(result.is_dynamic);
            assert_eq!(result.names, None);
        }
    }
}
