//! Shared utilities for visitor implementations

use ruff_python_ast::{Expr, ExprList, ExprStringLiteral, ExprTuple};

/// Result of extracting exports from an expression
#[derive(Debug)]
pub struct ExtractedExports {
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
/// - `ExtractedExports` with is_dynamic=true if any element is not a string literal
pub fn extract_string_list_from_expr(expr: &Expr) -> ExtractedExports {
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
    let mut names = Vec::new();

    for elt in elts {
        if let Expr::StringLiteral(ExprStringLiteral { value, .. }) = elt {
            names.push(value.to_str().to_string());
        } else {
            // Non-literal element found
            return ExtractedExports {
                names: None,
                is_dynamic: true,
            };
        }
    }

    ExtractedExports {
        names: Some(names),
        is_dynamic: false,
    }
}

/// Extract a string value from an expression if it's a string literal
pub fn extract_string_from_expr(expr: &Expr) -> Option<String> {
    if let Expr::StringLiteral(string_lit) = expr {
        Some(string_lit.value.to_str().to_string())
    } else {
        None
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

        if let Some(stmt) = module.body.first()
            && let ruff_python_ast::Stmt::Expr(expr_stmt) = stmt
        {
            let result = extract_string_list_from_expr(&expr_stmt.value);
            assert!(!result.is_dynamic);
            assert_eq!(
                result.names,
                Some(vec![
                    "foo".to_string(),
                    "bar".to_string(),
                    "baz".to_string()
                ])
            );
        }
    }

    #[test]
    fn test_extract_string_list_with_non_literal() {
        let code = r#"["foo", some_var, "baz"]"#;
        let parsed = parse_module(code).expect("Failed to parse");
        let module = parsed.into_syntax();

        if let Some(stmt) = module.body.first()
            && let ruff_python_ast::Stmt::Expr(expr_stmt) = stmt
        {
            let result = extract_string_list_from_expr(&expr_stmt.value);
            assert!(result.is_dynamic);
            assert_eq!(result.names, None);
        }
    }
}
