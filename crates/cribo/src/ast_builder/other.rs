//! Auxiliary AST node factory functions
//!
//! This module provides factory functions for creating auxiliary AST nodes such as
//! aliases, keywords, arguments, and exception handlers. All nodes are created with
//! `TextRange::default()` and `AtomicNodeIndex::dummy()` to indicate their synthetic nature.

use ruff_python_ast::{
    Alias, AtomicNodeIndex, ExceptHandler, ExceptHandlerExceptHandler, Expr, Keyword, Stmt,
};
use ruff_text_size::TextRange;

/// Creates an alias node for import statements.
///
/// # Arguments
/// * `name` - The name being imported
/// * `asname` - The alias name (None if no alias)
///
/// # Example
/// ```rust
/// // Creates: `foo as bar`
/// let alias = alias("foo", Some("bar"));
///
/// // Creates: `baz` (no alias)
/// let alias = alias("baz", None);
/// ```
pub fn alias(name: &str, asname: Option<&str>) -> Alias {
    use ruff_python_ast::Identifier;
    Alias {
        name: Identifier::new(name, TextRange::default()),
        asname: asname.map(|s| Identifier::new(s, TextRange::default())),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    }
}

/// Creates a keyword argument node for function calls.
///
/// # Arguments
/// * `arg` - The keyword argument name
/// * `value` - The argument value
///
/// # Example
/// ```rust
/// // Creates: `key=value`
/// use crate::ast_builder::expressions;
/// let keyword = keyword("key", expressions::string_literal("value"));
/// ```
pub fn keyword(arg: &str, value: Expr) -> Keyword {
    use ruff_python_ast::Identifier;
    Keyword {
        arg: Some(Identifier::new(arg, TextRange::default())),
        value,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    }
}

/// Creates a keyword unpacking node for function calls (for `**kwargs` patterns).
///
/// # Arguments
/// * `value` - The expression being unpacked
///
/// # Example
/// ```rust
/// // Creates: `**kwargs`
/// use crate::ast_builder::expressions;
/// let kwargs_expr = expressions::name("kwargs", ruff_python_ast::ExprContext::Load);
/// let keyword = keyword_unpack(kwargs_expr);
/// ```
pub fn keyword_unpack(value: Expr) -> Keyword {
    Keyword {
        arg: None, // None indicates **kwargs unpacking
        value,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    }
}

// Note: The Arguments type in ruff_python_ast is for call arguments, not function parameters
// Function parameters would use a different structure (Parameters)
// For now, we'll skip implementing this until it's needed

/// Creates an exception handler node for try statements.
///
/// # Arguments
/// * `type_` - The exception type to catch (None catches all)
/// * `name` - The variable name to bind the exception to
/// * `body` - The handler body statements
///
/// # Example
/// ```rust
/// // Creates: `except ValueError as e: pass`
/// use crate::ast_builder::{expressions, statements};
/// let handler = except_handler(
///     Some(expressions::name(
///         "ValueError",
///         ruff_python_ast::ExprContext::Load,
///     )),
///     Some("e"),
///     vec![statements::pass()],
/// );
///
/// // Creates: `except: pass` (catch all)
/// let handler = except_handler(None, None, vec![statements::pass()]);
/// ```
pub fn except_handler(type_: Option<Expr>, name: Option<&str>, body: Vec<Stmt>) -> ExceptHandler {
    use ruff_python_ast::Identifier;
    ExceptHandler::ExceptHandler(ExceptHandlerExceptHandler {
        type_: type_.map(Box::new),
        name: name.map(|s| Identifier::new(s, TextRange::default())),
        body,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}
