//! Statement AST node factory functions
//!
//! This module provides factory functions for creating various types of statement AST nodes.
//! All statements are created with `TextRange::default()` and `AtomicNodeIndex::NONE`
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
pub(crate) fn assign(targets: Vec<Expr>, value: Expr) -> Stmt {
    Stmt::Assign(StmtAssign {
        targets,
        value: Box::new(value),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
    })
}

/// Attach original-statement provenance to synthesized assignments.
///
/// Import rewriting often expands one import into several assignments. Those
/// assignments execute at the import site, so preserving its range and node
/// index lets source-map extraction map failures in generated initializer calls
/// back to the user-written import line.
pub(crate) fn inherit_assignment_provenance(
    statements: &mut [Stmt],
    range: TextRange,
    node_index: ruff_python_ast::NodeIndex,
) {
    for statement in statements {
        if let Stmt::Assign(assign) = statement {
            assign.range = range;
            assign.node_index.set(node_index);
        }
    }
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
pub(crate) fn simple_assign(target: &str, value: Expr) -> Stmt {
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
pub(crate) fn assign_attribute(obj_name: &str, attr_name: &str, value: Expr) -> Stmt {
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
pub(crate) fn assign_attribute_path(path: &str, value: Expr) -> Stmt {
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
pub(crate) fn expr(expr: Expr) -> Stmt {
    Stmt::Expr(StmtExpr {
        value: Box::new(expr),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
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
pub(crate) fn import(names: Vec<Alias>) -> Stmt {
    Stmt::Import(StmtImport {
        names,
        is_lazy: false,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
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
pub(crate) fn import_aliased(module_name: &str, alias_name: &str) -> Stmt {
    Stmt::Import(StmtImport {
        node_index: AtomicNodeIndex::NONE,
        names: vec![super::other::alias(module_name, Some(alias_name))],
        is_lazy: false,
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
pub(crate) fn import_from(module: Option<&str>, names: Vec<Alias>, level: u32) -> Stmt {
    Stmt::ImportFrom(StmtImportFrom {
        module: module.map(|s| Identifier::new(s, TextRange::default())),
        names,
        level,
        is_lazy: false,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
    })
}

/// Creates a pass statement node.
///
/// # Example
/// ```rust
/// // Creates: `pass`
/// let stmt = pass();
/// ```
pub(crate) fn pass() -> Stmt {
    Stmt::Pass(StmtPass {
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
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
pub(crate) fn return_stmt(value: Option<Expr>) -> Stmt {
    Stmt::Return(StmtReturn {
        value: value.map(Box::new),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
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
pub(crate) fn raise(exc: Option<Expr>, cause: Option<Expr>) -> Stmt {
    Stmt::Raise(StmtRaise {
        exc: exc.map(Box::new),
        cause: cause.map(Box::new),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
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
pub(crate) fn global(names: Vec<&str>) -> Stmt {
    Stmt::Global(StmtGlobal {
        names: names
            .into_iter()
            .map(|s| Identifier::new(s, TextRange::default()))
            .collect(),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
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
///     node_index: AtomicNodeIndex::NONE,
///     posonlyargs: vec![],
///     args: vec![],
///     vararg: None,
///     kwonlyargs: vec![],
///     kwarg: None,
///     range: TextRange::default(),
/// };
/// let stmt = function_def("my_func", params, vec![pass()], vec![], None, false);
/// ```
pub(crate) fn function_def(
    name: &str,
    parameters: Parameters,
    body: Vec<Stmt>,
    decorator_list: Vec<Decorator>,
    returns: Option<Expr>,
    is_async: bool,
) -> Stmt {
    Stmt::FunctionDef(StmtFunctionDef {
        node_index: AtomicNodeIndex::NONE,
        name: Identifier::new(name, TextRange::default()),
        type_params: None,
        parameters: Box::new(parameters),
        returns: returns.map(Box::new),
        body: body.into(),
        decorator_list: decorator_list.into(),
        is_async,
        range: TextRange::default(),
    })
}

/// Creates a statement to assign a string literal to an object's attribute.
/// e.g., `obj.attr = "value"`
pub(crate) fn set_string_attribute(obj_name: &str, attr_name: &str, value: &str) -> Stmt {
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
pub(crate) fn set_list_attribute(obj_name: &str, attr_name: &str, values: &[&str]) -> Stmt {
    let list_elements: Vec<Expr> = values
        .iter()
        .map(|value| expressions::string_literal(value))
        .collect();
    assign(
        vec![expressions::attribute(
            expressions::name(obj_name, ExprContext::Load),
            attr_name,
            ExprContext::Store,
        )],
        expressions::list(list_elements, ExprContext::Load),
    )
}

/// Create a for loop statement
pub(crate) fn for_loop(target: &str, iter: Expr, body: Vec<Stmt>, orelse: Vec<Stmt>) -> Stmt {
    Stmt::For(ruff_python_ast::StmtFor {
        node_index: AtomicNodeIndex::NONE,
        target: Box::new(expressions::name(target, ExprContext::Store)),
        iter: Box::new(iter),
        body: body.into(),
        orelse: orelse.into(),
        is_async: false,
        range: TextRange::default(),
    })
}

/// Create an if statement
pub(crate) fn if_stmt(condition: Expr, body: Vec<Stmt>, orelse: Vec<Stmt>) -> Stmt {
    Stmt::If(ruff_python_ast::StmtIf {
        node_index: AtomicNodeIndex::NONE,
        test: Box::new(condition),
        body: body.into(),
        elif_else_clauses: if orelse.is_empty() {
            vec![]
        } else {
            vec![ruff_python_ast::ElifElseClause {
                node_index: AtomicNodeIndex::NONE,
                test: None,
                body: orelse.into(),
                range: TextRange::default(),
            }]
        },
        range: TextRange::default(),
    })
}

/// Create a subscript assignment statement: target[key] = value
pub(crate) fn subscript_assign(target: Expr, key: Expr, value: Expr) -> Stmt {
    Stmt::Assign(StmtAssign {
        node_index: AtomicNodeIndex::NONE,
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
pub(crate) fn try_stmt(
    body: Vec<Stmt>,
    handlers: Vec<ExceptHandler>,
    orelse: Vec<Stmt>,
    finalbody: Vec<Stmt>,
) -> Stmt {
    Stmt::Try(StmtTry {
        body: body.into(),
        handlers,
        orelse: orelse.into(),
        finalbody: finalbody.into(),
        is_star: false,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
    })
}

/// Stamp the provider module's identity onto every definition executed inside
/// a bundled suite, by appending an INNERMOST decorator to each function and
/// class definition found at any nesting depth — inside compound suites
/// (`if`/`for`/`while`/`with`/`try`/`match`) and inside other definitions'
/// bodies (methods, factory-created inner functions, nested classes):
///
/// ```python
/// @lambda _cribo_f, _cribo_setattr=setattr: (_cribo_setattr(_cribo_f, '__module__', 'records'), _cribo_f)[1]
/// def __init__(self): ...
/// ```
///
/// Functions and classes created while bundled code executes read the BUNDLE
/// entry's `__name__` for their `__module__`; post-definition stamps can only
/// correct the objects bound at module scope. The innermost decorator runs
/// FIRST (before any user decorator observes the object), so decorator-time
/// reads of `f.__module__` and later introspection both see the provider
/// module. `setattr` is captured as a parameter default when the lambda is
/// created.
///
/// Callers pass a CLASS body or a FUNCTION body — never the module-level
/// statement list, whose definitions receive guarded post-definition stamps
/// instead (pre-stamping them would defeat the imported-callable provenance
/// check those guards perform).
pub(crate) fn stamp_nested_definitions(suite: &mut [Stmt], module_name: &str) {
    for stmt in suite {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                stamp_nested_definitions(&mut func_def.body, module_name);
                func_def
                    .decorator_list
                    .push(module_stamp_decorator(module_name));
            }
            Stmt::ClassDef(class_def) => {
                stamp_nested_definitions(&mut class_def.body, module_name);
                class_def
                    .decorator_list
                    .push(module_stamp_decorator(module_name));
            }
            Stmt::If(if_stmt) => {
                stamp_nested_definitions(&mut if_stmt.body, module_name);
                for clause in &mut if_stmt.elif_else_clauses {
                    stamp_nested_definitions(&mut clause.body, module_name);
                }
            }
            Stmt::For(for_stmt) => {
                stamp_nested_definitions(&mut for_stmt.body, module_name);
                stamp_nested_definitions(&mut for_stmt.orelse, module_name);
            }
            Stmt::While(while_stmt) => {
                stamp_nested_definitions(&mut while_stmt.body, module_name);
                stamp_nested_definitions(&mut while_stmt.orelse, module_name);
            }
            Stmt::With(with_stmt) => {
                stamp_nested_definitions(&mut with_stmt.body, module_name);
            }
            Stmt::Try(try_stmt) => {
                stamp_nested_definitions(&mut try_stmt.body, module_name);
                for handler in &mut try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(handler) = handler;
                    stamp_nested_definitions(&mut handler.body, module_name);
                }
                stamp_nested_definitions(&mut try_stmt.orelse, module_name);
                stamp_nested_definitions(&mut try_stmt.finalbody, module_name);
            }
            Stmt::Match(match_stmt) => {
                for case in &mut match_stmt.cases {
                    stamp_nested_definitions(&mut case.body, module_name);
                }
            }
            _ => {}
        }
    }
}

/// Build the innermost `__module__`-stamping decorator used by
/// [`stamp_nested_definitions`].
fn module_stamp_decorator(module_name: &str) -> Decorator {
    use ruff_python_ast::{ExprLambda, Parameter, ParameterWithDefault};

    let parameter = |name: &str, default: Option<Expr>| ParameterWithDefault {
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
        parameter: Parameter {
            range: TextRange::default(),
            node_index: AtomicNodeIndex::NONE,
            name: Identifier::new(name, TextRange::default()),
            annotation: None,
        },
        default: default.map(Box::new),
    };
    let parameters = Parameters {
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
        posonlyargs: vec![].into(),
        args: vec![
            parameter("_cribo_f", None),
            parameter(
                "_cribo_setattr",
                Some(expressions::name("setattr", ExprContext::Load)),
            ),
        ]
        .into(),
        vararg: None,
        kwonlyargs: vec![].into(),
        kwarg: None,
    };
    let stamp_call = expressions::call(
        expressions::name("_cribo_setattr", ExprContext::Load),
        vec![
            expressions::name("_cribo_f", ExprContext::Load),
            expressions::string_literal("__module__"),
            expressions::string_literal(module_name),
        ],
        vec![],
    );
    let body = expressions::subscript(
        expressions::tuple(vec![
            stamp_call,
            expressions::name("_cribo_f", ExprContext::Load),
        ]),
        expressions::integer_literal(1),
        ExprContext::Load,
    );
    Decorator {
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
        expression: Expr::Lambda(ExprLambda {
            range: TextRange::default(),
            node_index: AtomicNodeIndex::NONE,
            parameters: Some(Box::new(parameters)),
            body: Box::new(body),
        }),
    }
}
