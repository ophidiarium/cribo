use ruff_python_ast::{AtomicNodeIndex, ExprContext, Identifier, Keyword, Stmt};
use ruff_text_size::TextRange;

use crate::ast_builder::{expressions, statements};
use crate::code_generator::module_registry::{
    get_init_function_name, sanitize_module_name_for_identifier,
};

/// Creates a complete wrapper module with namespace, init function, and __init__ assignment
/// Returns a vector of statements that should be added to the bundle in order
pub fn create_wrapper_module(
    module_name: &str,
    synthetic_name: &str,
    init_function_body: Stmt,
    is_package: bool,
) -> Vec<Stmt> {
    let mut stmts = Vec::new();

    let module_var = sanitize_module_name_for_identifier(module_name);
    let init_func_name = get_init_function_name(synthetic_name);

    // 1. Create namespace with __initializing__ and __initialized__ flags
    // module_var = types.SimpleNamespace(__name__='...', __initializing__=False, __initialized__=False)
    let mut kwargs = vec![
        // __name__ = 'module_name'
        Keyword {
            node_index: AtomicNodeIndex::dummy(),
            arg: Some(Identifier::new("__name__", TextRange::default())),
            value: expressions::string_literal(module_name),
            range: TextRange::default(),
        },
        // __initializing__ = False
        Keyword {
            node_index: AtomicNodeIndex::dummy(),
            arg: Some(Identifier::new("__initializing__", TextRange::default())),
            value: expressions::name("False", ExprContext::Load),
            range: TextRange::default(),
        },
        // __initialized__ = False
        Keyword {
            node_index: AtomicNodeIndex::dummy(),
            arg: Some(Identifier::new("__initialized__", TextRange::default())),
            value: expressions::name("False", ExprContext::Load),
            range: TextRange::default(),
        },
    ];

    // Add __path__ for packages
    if is_package {
        kwargs.push(Keyword {
            node_index: AtomicNodeIndex::dummy(),
            arg: Some(Identifier::new("__path__", TextRange::default())),
            value: expressions::list(vec![], ExprContext::Load),
            range: TextRange::default(),
        });
    }

    let namespace_stmt = statements::simple_assign(
        &module_var,
        expressions::call(expressions::simple_namespace_ctor(), vec![], kwargs),
    );
    stmts.push(namespace_stmt);

    // 2. Add the init function definition
    stmts.push(init_function_body);

    // 3. Attach the init function to the module's __init__ attribute
    let attach_init = statements::assign(
        vec![expressions::attribute(
            expressions::name(&module_var, ExprContext::Load),
            "__init__",
            ExprContext::Store,
        )],
        expressions::name(&init_func_name, ExprContext::Load),
    );
    stmts.push(attach_init);

    stmts
}
