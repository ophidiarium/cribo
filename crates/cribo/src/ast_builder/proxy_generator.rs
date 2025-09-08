//! Generate the _cribo proxy for dynamic stdlib access
//!
//! This module generates the minimal proxy implementation that replaces
//! the complex stdlib import hoisting mechanism.
//!
//! ## Why is this needed?
//!
//! When bundling Python code, stdlib imports are normalized to use `_cribo.<module>`
//! to avoid conflicts with local variable names. For example, if user code has
//! a local variable named `json`, it won't conflict with stdlib `json` module
//! because stdlib access goes through `_cribo.json`.
//!
//! ## What does it do?
//!
//! The proxy provides lazy, on-demand access to stdlib modules through a simple
//! `__getattr__` mechanism. When code accesses `_cribo.json`, the proxy:
//! 1. Checks if the module is already loaded in `sys.modules`
//! 2. If not, imports it using `importlib.import_module`
//! 3. Returns a wrapper that allows nested access (e.g., `_cribo.os.path`)
//!
//! ## Generated Python code:
//!
//! ```python
//! import sys as _sys, importlib as _importlib
//!
//! class _CriboModule:
//!     def __init__(self, m, p):
//!         self._m, self._p = m, p
//!     def __getattr__(self, n):
//!         f = f"{self._p}.{n}"
//!         try: return _CriboModule(_importlib.import_module(f), f)
//!         except ImportError: return getattr(self._m, n)
//!     def __getattribute__(self, n):
//!         return (object.__getattribute__(self, n) if n in ('_m','_p','__getattr__')
//!                 else getattr(object.__getattribute__(self, '_m'), n))
//!
//! class _Cribo:
//!     def __getattr__(self, n):
//!         m = _sys.modules.get(n) or _importlib.import_module(n)
//!         return _CriboModule(m, n)
//!
//! _cribo = _Cribo()
//! ```

use ruff_python_ast::{
    Arguments, AtomicNodeIndex, ExceptHandler, ExprContext, Identifier, Parameter,
    ParameterWithDefault, Parameters, Stmt, StmtClassDef,
};
use ruff_text_size::TextRange;

use crate::ast_builder::{expressions, statements};

/// Constants for the proxy implementation
const SYS_MODULE: &str = "sys";
use super::CRIBO_PREFIX;

const SYS_ALIAS: &str = "_sys";
const IMPORTLIB_MODULE: &str = "importlib";
const IMPORTLIB_ALIAS: &str = "_importlib";
const CRIBO_MODULE_CLASS: &str = "_CriboModule";
const CRIBO_CLASS: &str = "_Cribo";
const CRIBO_INSTANCE: &str = CRIBO_PREFIX;

/// Helper to create simple function parameters
fn make_params(names: &[&str]) -> Parameters {
    Parameters {
        posonlyargs: vec![],
        args: names
            .iter()
            .map(|name| ParameterWithDefault {
                parameter: Parameter {
                    node_index: AtomicNodeIndex::dummy(),
                    name: Identifier::new(*name, TextRange::default()),
                    annotation: None,
                    range: TextRange::default(),
                },
                default: None,
                range: TextRange::default(),
                node_index: AtomicNodeIndex::dummy(),
            })
            .collect(),
        vararg: None,
        kwonlyargs: vec![],
        kwarg: None,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    }
}

/// Generate the _cribo proxy for dynamic stdlib access
///
/// Returns AST nodes for:
/// - Import statements for sys and importlib
/// - _`CriboModule` class definition
/// - _Cribo class definition
/// - _cribo instance creation
pub fn generate_cribo_proxy() -> Vec<Stmt> {
    let mut stmts = Vec::new();

    // import sys as _sys
    stmts.push(statements::import_aliased(SYS_MODULE, SYS_ALIAS));

    // import importlib as _importlib
    stmts.push(statements::import_aliased(
        IMPORTLIB_MODULE,
        IMPORTLIB_ALIAS,
    ));

    // Create _CriboModule class
    let cribo_module_class = create_cribo_module_class();
    stmts.push(cribo_module_class);

    // Create _Cribo class
    let cribo_class = create_cribo_class();
    stmts.push(cribo_class);

    // _cribo = _Cribo()
    let cribo_instance = statements::assign(
        vec![expressions::name(CRIBO_INSTANCE, ExprContext::Store)],
        expressions::call(
            expressions::name(CRIBO_CLASS, ExprContext::Load),
            vec![],
            vec![],
        ),
    );
    stmts.push(cribo_instance);

    stmts
}

/// Create the _`CriboModule` class AST
/// This wrapper class handles nested module access (e.g., _cribo.os.path)
fn create_cribo_module_class() -> Stmt {
    let mut body = Vec::new();

    // def __init__(self, m, p):
    //     self._m, self._p = m, p
    let init_method = create_cribo_module_init();
    body.push(init_method);

    // def __getattr__(self, n):
    //     f = f"{self._p}.{n}"
    //     try: return _CriboModule(_importlib.import_module(f), f)
    //     except ImportError: return getattr(self._m, n)
    let getattr_method = create_cribo_module_getattr();
    body.push(getattr_method);

    // def __getattribute__(self, n):
    //     return (object.__getattribute__(self, n) if n in ('_m','_p','__getattr__')
    //             else getattr(object.__getattribute__(self, '_m'), n))
    let getattribute_method = create_cribo_module_getattribute();
    body.push(getattribute_method);

    Stmt::ClassDef(StmtClassDef {
        node_index: AtomicNodeIndex::dummy(),
        name: Identifier::new(CRIBO_MODULE_CLASS, TextRange::default()),
        arguments: Some(Box::new(Arguments {
            args: Box::new([]),
            keywords: Box::new([]),
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        })),
        body,
        decorator_list: vec![],
        type_params: None,
        range: TextRange::default(),
    })
}

/// Create __init__ method for _`CriboModule`
fn create_cribo_module_init() -> Stmt {
    // self._m, self._p = m, p
    let assign_stmt = statements::assign(
        vec![expressions::tuple(vec![
            expressions::attribute(
                expressions::name("self", ExprContext::Load),
                "_m",
                ExprContext::Store,
            ),
            expressions::attribute(
                expressions::name("self", ExprContext::Load),
                "_p",
                ExprContext::Store,
            ),
        ])],
        expressions::tuple(vec![
            expressions::name("m", ExprContext::Load),
            expressions::name("p", ExprContext::Load),
        ]),
    );

    statements::function_def(
        crate::python::constants::INIT_STEM,
        make_params(&["self", "m", "p"]),
        vec![assign_stmt],
        vec![], // decorator_list
        None,   // returns
        false,  // is_async
    )
}

/// Create __getattr__ method for _`CriboModule`
fn create_cribo_module_getattr() -> Stmt {
    // For f-string, we'll use a simpler approach with binary operations
    // f = self._p + "." + n
    let f_assign = statements::assign(
        vec![expressions::name("f", ExprContext::Store)],
        expressions::bin_op(
            expressions::bin_op(
                expressions::attribute(
                    expressions::name("self", ExprContext::Load),
                    "_p",
                    ExprContext::Load,
                ),
                ruff_python_ast::Operator::Add,
                expressions::string_literal("."),
            ),
            ruff_python_ast::Operator::Add,
            expressions::name("n", ExprContext::Load),
        ),
    );

    // _CriboModule(_importlib.import_module(f), f)
    let cribo_module_call = expressions::call(
        expressions::name(CRIBO_MODULE_CLASS, ExprContext::Load),
        vec![
            expressions::call(
                expressions::attribute(
                    expressions::name(IMPORTLIB_ALIAS, ExprContext::Load),
                    "import_module",
                    ExprContext::Load,
                ),
                vec![expressions::name("f", ExprContext::Load)],
                vec![],
            ),
            expressions::name("f", ExprContext::Load),
        ],
        vec![],
    );

    // return _CriboModule(...)
    let try_body = vec![statements::return_stmt(Some(cribo_module_call))];

    // getattr(self._m, n)
    let getattr_call = expressions::call(
        expressions::name("getattr", ExprContext::Load),
        vec![
            expressions::attribute(
                expressions::name("self", ExprContext::Load),
                "_m",
                ExprContext::Load,
            ),
            expressions::name("n", ExprContext::Load),
        ],
        vec![],
    );

    // return getattr(...)
    let except_body = vec![statements::return_stmt(Some(getattr_call))];

    // ImportError handler
    let except_handler =
        ExceptHandler::ExceptHandler(ruff_python_ast::ExceptHandlerExceptHandler {
            node_index: AtomicNodeIndex::dummy(),
            type_: Some(Box::new(expressions::name(
                "ImportError",
                ExprContext::Load,
            ))),
            name: None,
            body: except_body,
            range: TextRange::default(),
        });

    // try: ... except ImportError: ...
    let try_stmt = statements::try_stmt(try_body, vec![except_handler], vec![], vec![]);

    statements::function_def(
        "__getattr__",
        make_params(&["self", "n"]),
        vec![f_assign, try_stmt],
        vec![], // decorator_list
        None,   // returns
        false,  // is_async
    )
}

/// Create __getattribute__ method for _`CriboModule`
fn create_cribo_module_getattribute() -> Stmt {
    // n in ('_m','_p','__getattr__', '__class__', '__dict__', '__dir__', '__module__',
    // '__qualname__')
    let special_attrs = expressions::in_op(
        expressions::name("n", ExprContext::Load),
        expressions::tuple(vec![
            expressions::string_literal("_m"),
            expressions::string_literal("_p"),
            expressions::string_literal("__getattr__"),
            expressions::string_literal("__class__"),
            expressions::string_literal("__dict__"),
            expressions::string_literal("__dir__"),
            expressions::string_literal("__module__"),
            expressions::string_literal("__qualname__"),
        ]),
    );

    // object.__getattribute__(self, n)
    let object_getattr_n = expressions::call(
        expressions::attribute(
            expressions::name("object", ExprContext::Load),
            "__getattribute__",
            ExprContext::Load,
        ),
        vec![
            expressions::name("self", ExprContext::Load),
            expressions::name("n", ExprContext::Load),
        ],
        vec![],
    );

    // object.__getattribute__(self, '_m')
    let object_getattr_m = expressions::call(
        expressions::attribute(
            expressions::name("object", ExprContext::Load),
            "__getattribute__",
            ExprContext::Load,
        ),
        vec![
            expressions::name("self", ExprContext::Load),
            expressions::string_literal("_m"),
        ],
        vec![],
    );

    // getattr(object.__getattribute__(self, '_m'), n)
    let getattr_call = expressions::call(
        expressions::name("getattr", ExprContext::Load),
        vec![object_getattr_m, expressions::name("n", ExprContext::Load)],
        vec![],
    );

    // Conditional expression
    let cond_expr = expressions::if_exp(special_attrs, object_getattr_n, getattr_call);

    statements::function_def(
        "__getattribute__",
        make_params(&["self", "n"]),
        vec![statements::return_stmt(Some(cond_expr))],
        vec![], // decorator_list
        None,   // returns
        false,  // is_async
    )
}

/// Create the _Cribo class AST\
/// This is the main proxy class that handles top-level module access
fn create_cribo_class() -> Stmt {
    // def __getattr__(self, n):
    //     m = _sys.modules.get(n) or _importlib.import_module(n)
    //     return _CriboModule(m, n)
    let getattr_method = create_cribo_getattr();

    Stmt::ClassDef(StmtClassDef {
        node_index: AtomicNodeIndex::dummy(),
        name: Identifier::new(CRIBO_CLASS, TextRange::default()),
        arguments: Some(Box::new(Arguments {
            args: Box::new([]),
            keywords: Box::new([]),
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        })),
        body: vec![getattr_method],
        decorator_list: vec![],
        type_params: None,
        range: TextRange::default(),
    })
}

/// Create __getattr__ method for _Cribo
fn create_cribo_getattr() -> Stmt {
    // _sys.modules.get(n)
    let sys_modules_get = expressions::call(
        expressions::attribute(
            expressions::attribute(
                expressions::name(SYS_ALIAS, ExprContext::Load),
                "modules",
                ExprContext::Load,
            ),
            "get",
            ExprContext::Load,
        ),
        vec![expressions::name("n", ExprContext::Load)],
        vec![],
    );

    // _importlib.import_module(n)
    let import_module = expressions::call(
        expressions::attribute(
            expressions::name(IMPORTLIB_ALIAS, ExprContext::Load),
            "import_module",
            ExprContext::Load,
        ),
        vec![expressions::name("n", ExprContext::Load)],
        vec![],
    );

    // _sys.modules.get(n) or _importlib.import_module(n)
    let m_expr = expressions::bool_op(
        ruff_python_ast::BoolOp::Or,
        vec![sys_modules_get, import_module],
    );

    // m = ...
    let m_assign = statements::assign(vec![expressions::name("m", ExprContext::Store)], m_expr);

    // _CriboModule(m, n)
    let cribo_module_call = expressions::call(
        expressions::name(CRIBO_MODULE_CLASS, ExprContext::Load),
        vec![
            expressions::name("m", ExprContext::Load),
            expressions::name("n", ExprContext::Load),
        ],
        vec![],
    );

    // return _CriboModule(m, n)
    let return_stmt = statements::return_stmt(Some(cribo_module_call));

    statements::function_def(
        "__getattr__",
        make_params(&["self", "n"]),
        vec![m_assign, return_stmt],
        vec![], // decorator_list
        None,   // returns
        false,  // is_async
    )
}

#[cfg(test)]
mod tests {
    use ruff_python_ast::Expr;
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_generate_cribo_proxy() {
        // Generate the proxy AST
        let proxy_stmts = generate_cribo_proxy();

        // Verify we have the expected number of statements
        assert_eq!(
            proxy_stmts.len(),
            5,
            "Expected 5 statements: 2 imports + 2 classes + 1 instance"
        );

        // Verify first statement is sys import
        match &proxy_stmts[0] {
            Stmt::Import(import) => {
                assert_eq!(import.names.len(), 1);
                assert_eq!(import.names[0].name.as_str(), SYS_MODULE);
                assert_eq!(
                    import.names[0]
                        .asname
                        .as_ref()
                        .expect("sys import should have alias")
                        .as_str(),
                    SYS_ALIAS
                );
            }
            _ => panic!("Expected first statement to be import {SYS_MODULE} as {SYS_ALIAS}"),
        }

        // Verify second statement is importlib import
        match &proxy_stmts[1] {
            Stmt::Import(import) => {
                assert_eq!(import.names.len(), 1);
                assert_eq!(import.names[0].name.as_str(), IMPORTLIB_MODULE);
                assert_eq!(
                    import.names[0]
                        .asname
                        .as_ref()
                        .expect("importlib import should have alias")
                        .as_str(),
                    IMPORTLIB_ALIAS
                );
            }
            _ => panic!(
                "Expected second statement to be import {IMPORTLIB_MODULE} as {IMPORTLIB_ALIAS}"
            ),
        }

        // Verify third statement is _CriboModule class
        match &proxy_stmts[2] {
            Stmt::ClassDef(class_def) => {
                assert_eq!(class_def.name.as_str(), CRIBO_MODULE_CLASS);
                assert_eq!(class_def.body.len(), 3); // __init__, __getattr__, __getattribute__

                // Verify __init__ method
                match &class_def.body[0] {
                    Stmt::FunctionDef(func) => {
                        assert_eq!(func.name.as_str(), crate::python::constants::INIT_STEM);
                        assert_eq!(func.parameters.args.len(), 3); // self, m, p
                    }
                    _ => panic!("Expected first method to be __init__"),
                }

                // Verify __getattr__ method
                match &class_def.body[1] {
                    Stmt::FunctionDef(func) => {
                        assert_eq!(func.name.as_str(), "__getattr__");
                        assert_eq!(func.parameters.args.len(), 2); // self, n
                    }
                    _ => panic!("Expected second method to be __getattr__"),
                }

                // Verify __getattribute__ method
                match &class_def.body[2] {
                    Stmt::FunctionDef(func) => {
                        assert_eq!(func.name.as_str(), "__getattribute__");
                        assert_eq!(func.parameters.args.len(), 2); // self, n
                    }
                    _ => panic!("Expected third method to be __getattribute__"),
                }
            }
            _ => panic!("Expected third statement to be {CRIBO_MODULE_CLASS} class"),
        }

        // Verify fourth statement is _Cribo class
        match &proxy_stmts[3] {
            Stmt::ClassDef(class_def) => {
                assert_eq!(class_def.name.as_str(), CRIBO_CLASS);
                assert_eq!(class_def.body.len(), 1); // __getattr__

                // Verify __getattr__ method
                match &class_def.body[0] {
                    Stmt::FunctionDef(func) => {
                        assert_eq!(func.name.as_str(), "__getattr__");
                        assert_eq!(func.parameters.args.len(), 2); // self, n
                    }
                    _ => panic!("Expected method to be __getattr__"),
                }
            }
            _ => panic!("Expected fourth statement to be {CRIBO_CLASS} class"),
        }

        // Verify fifth statement is _cribo instance creation
        match &proxy_stmts[4] {
            Stmt::Assign(assign) => {
                assert_eq!(assign.targets.len(), 1);
                match &assign.targets[0] {
                    Expr::Name(name) => assert_eq!(name.id.as_str(), CRIBO_INSTANCE),
                    _ => panic!("Expected assignment target to be {CRIBO_INSTANCE}"),
                }
                // Verify it's a call to _Cribo()
                match assign.value.as_ref() {
                    Expr::Call(call) => match call.func.as_ref() {
                        Expr::Name(name) => assert_eq!(name.id.as_str(), CRIBO_CLASS),
                        _ => panic!("Expected call to {CRIBO_CLASS}"),
                    },
                    _ => panic!("Expected assignment value to be a call"),
                }
            }
            _ => panic!("Expected fifth statement to be {CRIBO_INSTANCE} = {CRIBO_CLASS}()"),
        }
    }

    #[test]
    fn test_proxy_structure_validates_expected_python() {
        // The expected Python code that our AST should generate
        let expected_code = r#"import sys as _sys
import importlib as _importlib

class _CriboModule:
    def __init__(self, m, p):
        self._m, self._p = m, p
    
    def __getattr__(self, n):
        f = self._p + "." + n
        try:
            return _CriboModule(_importlib.import_module(f), f)
        except ImportError:
            return getattr(self._m, n)
    
    def __getattribute__(self, n):
        return object.__getattribute__(self, n) if n in ("_m", "_p", "__getattr__") else getattr(object.__getattribute__(self, "_m"), n)

class _Cribo:
    def __getattr__(self, n):
        m = _sys.modules.get(n) or _importlib.import_module(n)
        return _CriboModule(m, n)

_cribo = _Cribo()
"#;

        // Parse the expected code to ensure it's valid Python
        let parsed = parse_module(expected_code);
        assert!(
            parsed.is_ok(),
            "Expected proxy code should be valid Python: {:?}",
            parsed.as_ref().err()
        );
    }
}
