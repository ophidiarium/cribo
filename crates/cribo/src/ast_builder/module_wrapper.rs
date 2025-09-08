use ruff_python_ast::{ExprContext, Stmt};

use crate::{
    ast_builder::{expressions, statements},
    code_generator::module_registry::sanitize_module_name_for_identifier,
    python::constants::INIT_STEM,
};

/// The __init__ attribute name for module initialization
const MODULE_INIT_ATTR: &str = INIT_STEM;

/// Creates just the init function and __init__ assignment statements for a module
/// Returns a vector containing the init function definition and the __init__ assignment
pub fn create_init_function_statements(
    module_name: &str,
    init_func_name: &str,
    init_function_body: Stmt,
) -> Vec<Stmt> {
    let mut stmts = Vec::new();

    let module_var = sanitize_module_name_for_identifier(module_name);
    // Add init function
    stmts.push(init_function_body);

    // Attach the init function to the module's __init__ attribute
    let attach_init = statements::assign(
        vec![expressions::attribute(
            expressions::name(&module_var, ExprContext::Load),
            MODULE_INIT_ATTR,
            ExprContext::Store,
        )],
        expressions::name(init_func_name, ExprContext::Load),
    );
    stmts.push(attach_init);

    stmts
}

/// Creates a complete wrapper module with namespace, init function, and __init__ assignment
/// Returns a vector of statements that should be added to the bundle in order
/// If `init_function_body` is None, only creates the namespace without init function
pub fn create_wrapper_module(
    module_name: &str,
    init_func_name: &str,
    init_function_body: Option<Stmt>,
    is_package: bool,
) -> Vec<Stmt> {
    let mut stmts = Vec::new();

    let module_var = sanitize_module_name_for_identifier(module_name);

    // 1. Create namespace with __initializing__ and __initialized__ flags
    // module_var = types.SimpleNamespace(__name__='...', __initializing__=False,
    // __initialized__=False)
    let mut kwargs = vec![
        expressions::keyword(Some("__name__"), expressions::string_literal(module_name)),
        expressions::keyword(Some("__initializing__"), expressions::bool_literal(false)),
        expressions::keyword(Some("__initialized__"), expressions::bool_literal(false)),
    ];

    // Add __path__ for packages
    if is_package {
        kwargs.push(expressions::keyword(
            Some("__path__"),
            expressions::list(vec![], ExprContext::Load),
        ));
    }

    let namespace_stmt = statements::simple_assign(
        &module_var,
        expressions::call(expressions::simple_namespace_ctor(), vec![], kwargs),
    );
    stmts.push(namespace_stmt);

    // 2. Add the init function definition and __init__ assignment if provided
    if let Some(init_body) = init_function_body {
        let init_stmts = create_init_function_statements(module_name, init_func_name, init_body);
        stmts.extend(init_stmts);
    }

    stmts
}

/// Creates a wrapper module initialization call statement.
///
/// This creates the standard pattern for initializing wrapper modules:
/// `module = module.__init__(module)`
///
/// # Arguments
/// * `module_name` - The sanitized module variable name
///
/// # Example
/// ```rust
/// // Creates: `compat = compat.__init__(compat)`
/// let stmt = create_wrapper_module_init_call("compat");
/// ```
pub fn create_wrapper_module_init_call(module_name: &str) -> Stmt {
    statements::assign(
        vec![expressions::name(module_name, ExprContext::Store)],
        expressions::call(
            expressions::attribute(
                expressions::name(module_name, ExprContext::Load),
                MODULE_INIT_ATTR,
                ExprContext::Load,
            ),
            vec![expressions::name(module_name, ExprContext::Load)],
            vec![],
        ),
    )
}
