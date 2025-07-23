//! Statement AST node factory functions
//!
//! This module provides factory functions for creating various types of statement AST nodes.
//! All statements are created with `TextRange::default()` and `AtomicNodeIndex::dummy()`
//! to indicate their synthetic nature.

use ruff_python_ast::{
    Alias, Arguments, AtomicNodeIndex, ExceptHandler, Expr, ExprContext, Stmt, StmtAssign,
    StmtClassDef, StmtExpr, StmtGlobal, StmtIf, StmtImport, StmtImportFrom, StmtPass, StmtRaise,
    StmtReturn, StmtTry,
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
    use ruff_python_ast::Identifier;
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
    use ruff_python_ast::Identifier;
    Stmt::Global(StmtGlobal {
        names: names
            .into_iter()
            .map(|s| Identifier::new(s, TextRange::default()))
            .collect(),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates an if statement node.
///
/// # Arguments
/// * `test` - The condition expression
/// * `body` - The statements to execute if the condition is true
/// * `orelse` - The statements to execute if the condition is false (optional)
///
/// # Example
/// ```rust
/// // Creates: `if condition: body_stmt else: else_stmt`
/// let condition = expressions::name("condition", ExprContext::Load);
/// let body_stmt = pass();
/// let else_stmt = pass();
/// let stmt = if_stmt(condition, vec![body_stmt], vec![else_stmt]);
/// ```
pub fn if_stmt(test: Expr, body: Vec<Stmt>, orelse: Vec<Stmt>) -> Stmt {
    use ruff_python_ast::ElifElseClause;

    let mut elif_else_clauses = Vec::new();

    // If there's an orelse, add it as an else clause
    if !orelse.is_empty() {
        elif_else_clauses.push(ElifElseClause {
            test: None, // None indicates else clause
            body: orelse,
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        });
    }

    Stmt::If(StmtIf {
        test: Box::new(test),
        body,
        elif_else_clauses,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a raise statement node.
///
/// # Arguments
/// * `exc` - The exception to raise (None for bare `raise`)
/// * `cause` - The exception cause (for `raise ... from ...`)
///
/// # Example
/// ```rust
/// // Creates: `raise ValueError("message")`
/// let exc = expressions::call(
///     expressions::name("ValueError", ExprContext::Load),
///     vec![expressions::string_literal("message")],
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

/// Creates a try statement node.
///
/// # Arguments
/// * `body` - The statements in the try block
/// * `handlers` - The exception handlers
/// * `orelse` - The else clause statements
/// * `finalbody` - The finally clause statements
///
/// # Example
/// ```rust
/// // Creates: `try: body except Exception: pass`
/// use crate::ast_builder::other;
/// let body_stmt = pass();
/// let handler = other::except_handler(
///     Some(expressions::name("Exception", ExprContext::Load)),
///     None,
///     vec![pass()],
/// );
/// let stmt = try_stmt(vec![body_stmt], vec![handler], vec![], vec![]);
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
        is_star: false, // Regular try, not try*
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a class definition statement node.
///
/// # Arguments
/// * `name` - The class name
/// * `arguments` - The class arguments (base classes and metaclass)
/// * `body` - The class body statements
///
/// # Example
/// ```rust
/// // Creates: `class MyClass: pass`
/// let stmt = class_def("MyClass", None, vec![pass()]);
/// ```
pub fn class_def(name: &str, arguments: Option<Arguments>, body: Vec<Stmt>) -> Stmt {
    use ruff_python_ast::Identifier;
    Stmt::ClassDef(StmtClassDef {
        name: Identifier::new(name, TextRange::default()),
        type_params: None, // Generic type parameters
        arguments: arguments.map(Box::new),
        body,
        decorator_list: Vec::new(),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}
