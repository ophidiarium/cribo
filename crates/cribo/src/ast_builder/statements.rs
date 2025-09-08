//! Statement AST node factory functions
//!
//! This module provides factory functions for creating various types of statement AST nodes.
//! All statements are created with `TextRange::default()` and `AtomicNodeIndex::dummy()`
//! to indicate their synthetic nature.

use ruff_python_ast::{
    Alias, AtomicNodeIndex, Decorator, ExceptHandler, Expr, ExprContext, Identifier, Parameters,
    Stmt, StmtAssign, StmtExpr, StmtFunctionDef, StmtGlobal, StmtImport, StmtImportFrom, StmtPass,
    StmtRaise, StmtReturn, StmtTry,
};
use ruff_text_size::TextRange;

use super::expressions;

/// Creates an assignment statement node.
///
/// # Arguments
/// * `targets` - The assignment targets (left-hand side)
/// * `value` - The assigned value (right-hand side)
///
/// # Example
/// ```rust
/// // Creates: `x = 42`
/// let target = expressions::name("x", ExprContext::Store);
/// let value = expressions::string_literal("42");
/// let stmt = assign(vec![target], value);
/// ```
pub fn assign(targets: Vec<Expr>, value: Expr) -> Stmt {
    Stmt::Assign(StmtAssign {
        targets,
        value: Box::new(value),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a simple assignment statement with a string target.
///
/// This is a convenience wrapper around `assign` for the common case
/// of assigning to a single variable name.
///
/// # Arguments
/// * `target` - The variable name to assign to
/// * `value` - The assigned value
///
/// # Example
/// ```rust
/// // Creates: `result = None`
/// let stmt = simple_assign("result", expressions::none_literal());
/// ```
pub fn simple_assign(target: &str, value: Expr) -> Stmt {
    let target_expr = expressions::name(target, ExprContext::Store);
    assign(vec![target_expr], value)
}

/// Creates an assignment statement that sets an attribute on an object.
///
/// This creates an assignment like `obj.attr = value`.
///
/// # Arguments
/// * `obj_name` - The name of the object
/// * `attr_name` - The attribute name to set
/// * `value` - The value to assign
///
/// # Example
/// ```rust
/// // Creates: `module.__name__ = "mymodule"`
/// let stmt = assign_attribute(
///     "module",
///     "__name__",
///     expressions::string_literal("mymodule"),
/// );
/// ```
pub fn assign_attribute(obj_name: &str, attr_name: &str, value: Expr) -> Stmt {
    assign(
        vec![expressions::attribute(
            expressions::name(obj_name, ExprContext::Load),
            attr_name,
            ExprContext::Store,
        )],
        value,
    )
}

/// Creates an assignment to a possibly dotted attribute path.
///
/// Builds a nested attribute target for arbitrary dotted parents, e.g.
/// `pkg.subpkg.module = value` or `obj.attr = value`.
pub fn assign_attribute_path(path: &str, value: Expr) -> Stmt {
    if let Some((parent, child)) = path.rsplit_once('.') {
        let mut parts = parent.split('.');
        let first = parts.next().expect("non-empty parent path");
        let mut obj = expressions::name(first, ExprContext::Load);
        for part in parts {
            obj = expressions::attribute(obj, part, ExprContext::Load);
        }
        let target = expressions::attribute(obj, child, ExprContext::Store);
        assign(vec![target], value)
    } else {
        simple_assign(path, value)
    }
}

/// Creates an expression statement node.
///
/// # Arguments
/// * `expr` - The expression to wrap in a statement
///
/// # Example
/// ```rust
/// // Creates: `func()`
/// let call_expr = expressions::call(expressions::name("func", ExprContext::Load), vec![], vec![]);
/// let stmt = expr(call_expr);
/// ```
pub fn expr(expr: Expr) -> Stmt {
    Stmt::Expr(StmtExpr {
        value: Box::new(expr),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates an import statement node.
///
/// # Arguments
/// * `names` - The imported names and their aliases
///
/// # Example
/// ```rust
/// // Creates: `import sys, os as operating_system`
/// use crate::ast_builder::other;
/// let aliases = vec![
///     other::alias("sys", None),
///     other::alias("os", Some("operating_system")),
/// ];
/// let stmt = import(aliases);
/// ```
pub fn import(names: Vec<Alias>) -> Stmt {
    Stmt::Import(StmtImport {
        names,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a simple import statement with an alias.
///
/// This is a convenience wrapper for creating an import with a single module
/// that is aliased to a different name.
///
/// # Arguments
/// * `module_name` - The module to import
/// * `alias_name` - The alias to use for the module
///
/// # Example
/// ```rust
/// // Creates: `import sys as _sys`
/// let stmt = import_aliased("sys", "_sys");
/// ```
pub fn import_aliased(module_name: &str, alias_name: &str) -> Stmt {
    Stmt::Import(StmtImport {
        node_index: AtomicNodeIndex::dummy(),
        names: vec![super::other::alias(module_name, Some(alias_name))],
        range: TextRange::default(),
    })
}

/// Creates an import from statement node.
///
/// # Arguments
/// * `module` - The module name to import from (None for relative imports without module)
/// * `names` - The imported names and their aliases
/// * `level` - The relative import level (0 for absolute, 1 for `.`, 2 for `..`, etc.)
///
/// # Example
/// ```rust
/// // Creates: `from foo import bar`
/// use crate::ast_builder::other;
/// let stmt = import_from(Some("foo"), vec![other::alias("bar", None)], 0);
///
/// // Creates: `from . import baz`
/// let stmt = import_from(None, vec![other::alias("baz", None)], 1);
///
/// // Creates: `from ..parent import something`
/// let stmt = import_from(Some("parent"), vec![other::alias("something", None)], 2);
/// ```
pub fn import_from(module: Option<&str>, names: Vec<Alias>, level: u32) -> Stmt {
    Stmt::ImportFrom(StmtImportFrom {
        module: module.map(|s| Identifier::new(s, TextRange::default())),
        names,
        level,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a pass statement node.
///
/// # Example
/// ```rust
/// // Creates: `pass`
/// let stmt = pass();
/// ```
pub fn pass() -> Stmt {
    Stmt::Pass(StmtPass {
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a return statement node.
///
/// # Arguments
/// * `value` - The return value (None for bare `return`)
///
/// # Example
/// ```rust
/// // Creates: `return 42`
/// let stmt = return_stmt(Some(expressions::string_literal("42")));
///
/// // Creates: `return`
/// let stmt = return_stmt(None);
/// ```
pub fn return_stmt(value: Option<Expr>) -> Stmt {
    Stmt::Return(StmtReturn {
        value: value.map(Box::new),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a raise statement node.
///
/// # Arguments
/// * `exc` - The exception to raise (None for bare `raise`)
/// * `cause` - The exception cause for `raise ... from ...` (None for no cause)
///
/// # Example
/// ```rust
/// // Creates: `raise ImportError("module not found")`
/// let exc = expressions::call(
///     expressions::name("ImportError", ExprContext::Load),
///     vec![expressions::string_literal("module not found")],
///     vec![],
/// );
/// let stmt = raise(Some(exc), None);
///
/// // Creates: `raise`
/// let stmt = raise(None, None);
/// ```
pub fn raise(exc: Option<Expr>, cause: Option<Expr>) -> Stmt {
    Stmt::Raise(StmtRaise {
        exc: exc.map(Box::new),
        cause: cause.map(Box::new),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a global statement node.
///
/// # Arguments
/// * `names` - The global variable names
///
/// # Example
/// ```rust
/// // Creates: `global x, y, z`
/// let stmt = global(vec!["x", "y", "z"]);
/// ```
pub fn global(names: Vec<&str>) -> Stmt {
    Stmt::Global(StmtGlobal {
        names: names
            .into_iter()
            .map(|s| Identifier::new(s, TextRange::default()))
            .collect(),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a function definition statement node.
///
/// # Arguments
/// * `name` - The function name
/// * `parameters` - The function parameters
/// * `body` - The function body statements
/// * `decorator_list` - The function decorators
/// * `returns` - The return type annotation (optional)
/// * `is_async` - Whether this is an async function
///
/// # Example
/// ```rust
/// // Creates: `def my_func(): pass`
/// let params = Parameters {
///     node_index: AtomicNodeIndex::dummy(),
///     posonlyargs: vec![],
///     args: vec![],
///     vararg: None,
///     kwonlyargs: vec![],
///     kwarg: None,
///     range: TextRange::default(),
/// };
/// let stmt = function_def("my_func", params, vec![pass()], vec![], None, false);
/// ```
pub fn function_def(
    name: &str,
    parameters: Parameters,
    body: Vec<Stmt>,
    decorator_list: Vec<Decorator>,
    returns: Option<Expr>,
    is_async: bool,
) -> Stmt {
    Stmt::FunctionDef(StmtFunctionDef {
        node_index: AtomicNodeIndex::dummy(),
        name: Identifier::new(name, TextRange::default()),
        type_params: None,
        parameters: Box::new(parameters),
        returns: returns.map(Box::new),
        body,
        decorator_list,
        is_async,
        range: TextRange::default(),
    })
}

/// Creates a statement to assign a string literal to an object's attribute.
/// e.g., `obj.attr = "value"`
pub fn set_string_attribute(obj_name: &str, attr_name: &str, value: &str) -> Stmt {
    assign(
        vec![expressions::attribute(
            expressions::name(obj_name, ExprContext::Load),
            attr_name,
            ExprContext::Store,
        )],
        expressions::string_literal(value),
    )
}

/// Creates a statement to assign a list of string literals to an object's attribute.
/// e.g., `obj.attr = ["item1", "item2", "item3"]`
pub fn set_list_attribute(obj_name: &str, attr_name: &str, values: &[&str]) -> Stmt {
    let list_elements: Vec<ruff_python_ast::Expr> = values
        .iter()
        .map(|value| expressions::string_literal(value))
        .collect();
    assign(
        vec![expressions::attribute(
            expressions::name(obj_name, ExprContext::Load),
            attr_name,
            ExprContext::Store,
        )],
        expressions::list(list_elements, ruff_python_ast::ExprContext::Load),
    )
}

/// Create a for loop statement
pub fn for_loop(target: &str, iter: Expr, body: Vec<Stmt>, orelse: Vec<Stmt>) -> Stmt {
    Stmt::For(ruff_python_ast::StmtFor {
        node_index: AtomicNodeIndex::dummy(),
        target: Box::new(expressions::name(target, ExprContext::Store)),
        iter: Box::new(iter),
        body,
        orelse,
        is_async: false,
        range: TextRange::default(),
    })
}

/// Create an if statement
pub fn if_stmt(condition: Expr, body: Vec<Stmt>, orelse: Vec<Stmt>) -> Stmt {
    Stmt::If(ruff_python_ast::StmtIf {
        node_index: AtomicNodeIndex::dummy(),
        test: Box::new(condition),
        body,
        elif_else_clauses: if orelse.is_empty() {
            vec![]
        } else {
            vec![ruff_python_ast::ElifElseClause {
                node_index: AtomicNodeIndex::dummy(),
                test: None,
                body: orelse,
                range: TextRange::default(),
            }]
        },
        range: TextRange::default(),
    })
}

/// Create a subscript assignment statement: target[key] = value
pub fn subscript_assign(target: Expr, key: Expr, value: Expr) -> Stmt {
    Stmt::Assign(ruff_python_ast::StmtAssign {
        node_index: AtomicNodeIndex::dummy(),
        targets: vec![expressions::subscript(target, key, ExprContext::Store)],
        value: Box::new(value),
        range: TextRange::default(),
    })
}

/// Creates a try-except statement node.
///
/// # Arguments
/// * `body` - The statements to try
/// * `handlers` - The exception handlers
/// * `orelse` - The else clause statements (executed if no exception)
/// * `finalbody` - The finally clause statements (always executed)
///
/// # Example
/// ```rust
/// // Creates: try: ... except ImportError: ...
/// let try_body = vec![...];
/// let except_handler = ExceptHandler::ExceptHandler(...);
/// let stmt = try_stmt(try_body, vec![except_handler], vec![], vec![]);
/// ```
pub fn try_stmt(
    body: Vec<Stmt>,
    handlers: Vec<ExceptHandler>,
    orelse: Vec<Stmt>,
    finalbody: Vec<Stmt>,
) -> Stmt {
    Stmt::Try(StmtTry {
        body,
        handlers,
        orelse,
        finalbody,
        is_star: false,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}
