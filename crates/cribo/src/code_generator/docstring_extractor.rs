//! Module docstring extraction utilities.
//!
//! This module provides functions for extracting docstrings from Python AST nodes,
//! particularly module-level docstrings for use in bundled code.

use ruff_python_ast::{Expr, ModModule, Stmt};

/// Extract the module-level docstring from a Python module AST.
///
/// Returns `Some(docstring)` if the module has a docstring (first statement is a string literal),
/// or `None` if there is no docstring.
///
/// # Examples
///
/// ```python
/// """This is a module docstring."""
/// import os
/// ```
///
/// The above module would return `Some("This is a module docstring.")`.
pub fn extract_module_docstring(module: &ModModule) -> Option<String> {
    // Module docstring is the first statement if it's a string literal expression
    if let Some(first_stmt) = module.body.first()
        && let Stmt::Expr(expr_stmt) = first_stmt
        && let Expr::StringLiteral(string_lit) = expr_stmt.value.as_ref()
    {
        // Convert the string literal value to a String
        let docstring = string_lit.value.to_str().to_string();
        return Some(docstring);
    }
    None
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_extract_module_docstring_simple() {
        let source = r#"
"""This is a module docstring."""

def foo():
    pass
"#;
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        assert_eq!(docstring, Some("This is a module docstring.".to_string()));
    }

    #[test]
    fn test_extract_module_docstring_multiline() {
        let source = r#"
"""This is a multiline docstring.

It has multiple lines and paragraphs.
"""

import os
"#;
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        assert!(docstring.is_some());
        let doc = docstring.expect("Docstring should be Some");
        assert!(doc.contains("multiline docstring"));
        assert!(doc.contains("multiple lines"));
    }

    #[test]
    fn test_extract_module_docstring_none() {
        let source = r"
import os

def foo():
    pass
";
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        assert_eq!(docstring, None);
    }

    #[test]
    fn test_extract_module_docstring_not_first() {
        let source = r#"
import os

"""This is not a module docstring."""

def foo():
    pass
"#;
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        // Should be None because it's not the first statement
        assert_eq!(docstring, None);
    }

    #[test]
    fn test_extract_module_docstring_single_quotes() {
        let source = r"
'''This is a module docstring with single quotes.'''

def foo():
    pass
";
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        assert_eq!(
            docstring,
            Some("This is a module docstring with single quotes.".to_string())
        );
    }

    #[test]
    fn test_extract_module_docstring_after_future_import() {
        let source = r#"
from __future__ import annotations

"""This is a module docstring after __future__ import."""

def foo():
    pass
"#;
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        // Docstring after __future__ import is not a module docstring
        // (it would be the second statement)
        assert_eq!(docstring, None);
    }

    #[test]
    fn test_extract_module_docstring_before_future_import() {
        let source = r#"
"""This is a module docstring before __future__ import."""

from __future__ import annotations

def foo():
    pass
"#;
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        // Module docstring can appear before __future__ import
        assert_eq!(
            docstring,
            Some("This is a module docstring before __future__ import.".to_string())
        );
    }

    #[test]
    fn test_extract_module_docstring_with_shebang() {
        // Note: Shebang is handled by the parser and not part of the AST
        // The docstring should still be the first statement
        let source = r#"#!/usr/bin/env python3
"""This is a module docstring after shebang."""

def foo():
    pass
"#;
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        assert_eq!(
            docstring,
            Some("This is a module docstring after shebang.".to_string())
        );
    }

    #[test]
    fn test_extract_module_docstring_with_shebang_and_encoding() {
        // Shebang and encoding declarations don't affect AST
        let source = r#"#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""This is a module docstring after shebang and encoding."""

import os
"#;
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        assert_eq!(
            docstring,
            Some("This is a module docstring after shebang and encoding.".to_string())
        );
    }

    #[test]
    fn test_extract_module_docstring_with_shebang_encoding_and_future() {
        // Complex real-world scenario
        let source = r#"#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Module docstring can be before or after __future__.

This tests the most common pattern in production code.
"""

from __future__ import annotations, division

import os
"#;
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        assert!(docstring.is_some());
        let doc = docstring.expect("Docstring should be Some");
        assert!(doc.contains("Module docstring can be before or after"));
        assert!(doc.contains("production code"));
    }

    #[test]
    fn test_extract_module_docstring_empty_module() {
        let source = "";
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        assert_eq!(docstring, None);
    }

    #[test]
    fn test_extract_module_docstring_only_future_import() {
        let source = r"
from __future__ import annotations
";
        let module = parse_module(source).expect("Failed to parse").into_syntax();
        let docstring = extract_module_docstring(&module);
        assert_eq!(docstring, None);
    }
}
