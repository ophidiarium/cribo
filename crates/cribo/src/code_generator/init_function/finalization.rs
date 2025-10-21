//! Finalization phase for init function transformation
//!
//! This phase builds the final function statement from the accumulated state.

use ruff_python_ast::{AtomicNodeIndex, ExprContext, Identifier};
use ruff_text_size::TextRange;

use super::{TransformError, state::InitFunctionState};
use crate::{
    ast_builder,
    code_generator::{
        bundler::Bundler, context::ModuleTransformContext, module_transformer::SELF_PARAM,
    },
};

/// Phase responsible for finalizing and building the init function statement
pub(crate) struct FinalizationPhase;

impl FinalizationPhase {
    /// Build the final function statement from accumulated state
    ///
    /// This phase:
    /// 1. Transforms `globals()` calls to module.__dict__
    /// 2. Transforms `locals()` calls to vars(self)
    /// 3. Marks the module as fully initialized (__initialized__ = True)
    /// 4. Clears the initializing flag (__initializing__ = False)
    /// 5. Returns the module object (return self)
    /// 6. Creates function parameters with 'self' parameter
    /// 7. Builds and returns the complete function definition
    ///
    /// Note: This phase consumes the state (takes ownership) as it's the final phase
    pub(crate) fn build_function_stmt(
        bundler: &Bundler<'_>,
        ctx: &ModuleTransformContext<'_>,
        mut state: InitFunctionState,
    ) -> Result<ruff_python_ast::Stmt, TransformError> {
        // Transform globals() calls to module.__dict__ in the entire body
        // For wrapper modules with circular dependencies, use the wrapper version that doesn't
        // recurse into function bodies since those functions will be called later when 'self' is
        // not in scope. For regular wrapper modules (with side effects but no circular
        // deps), use normal transformation.
        log::debug!(
            "Transforming globals/locals for module '{}', is_wrapper_body={}, \
             is_in_circular_deps={}",
            ctx.module_name,
            ctx.is_wrapper_body,
            ctx.is_in_circular_deps
        );
        for stmt in &mut state.body {
            if ctx.is_in_circular_deps {
                // Module is in circular dependencies - don't transform globals() inside functions
                crate::code_generator::globals::transform_globals_in_stmt_wrapper(stmt, SELF_PARAM);
            } else {
                // Regular module or wrapper without circular deps - transform globals() everywhere
                crate::code_generator::globals::transform_globals_in_stmt(stmt, SELF_PARAM);
            }
            // Transform locals() calls to vars(self) in the entire body
            crate::code_generator::globals::transform_locals_in_stmt(stmt, SELF_PARAM);
        }

        // Mark as fully initialized (module is now fully populated)
        // self.__initialized__ = True  (set this first!)
        // self.__initializing__ = False
        state.body.push(ast_builder::statements::assign_attribute(
            SELF_PARAM,
            "__initialized__",
            ast_builder::expressions::bool_literal(true),
        ));
        state.body.push(ast_builder::statements::assign_attribute(
            SELF_PARAM,
            "__initializing__",
            ast_builder::expressions::bool_literal(false),
        ));

        // Return the module object (self)
        state.body.push(ast_builder::statements::return_stmt(Some(
            ast_builder::expressions::name(SELF_PARAM, ExprContext::Load),
        )));

        // Create the init function parameters with 'self' parameter
        let self_param = ruff_python_ast::ParameterWithDefault {
            range: TextRange::default(),
            parameter: ruff_python_ast::Parameter {
                range: TextRange::default(),
                name: Identifier::new(SELF_PARAM, TextRange::default()),
                annotation: None,
                node_index: AtomicNodeIndex::NONE,
            },
            default: None,
            node_index: AtomicNodeIndex::NONE,
        };

        let parameters = ruff_python_ast::Parameters {
            node_index: AtomicNodeIndex::NONE,
            posonlyargs: vec![],
            args: vec![self_param],
            vararg: None,
            kwonlyargs: vec![],
            kwarg: None,
            range: TextRange::default(),
        };

        // Get the init function name from the bundler
        let module_id = bundler.get_module_id(ctx.module_name).ok_or_else(|| {
            TransformError::ModuleIdNotFound {
                module_name: ctx.module_name.to_owned(),
            }
        })?;
        let init_func_name = bundler
            .module_init_functions
            .get(&module_id)
            .ok_or_else(|| TransformError::InitFunctionNotFound {
                module_id: module_id.to_string(),
            })?;

        // No decorator - we manage initialization ourselves
        let function_stmt = ast_builder::statements::function_def(
            init_func_name,
            parameters,
            state.body,
            vec![], // No decorators
            None,   // No return type annotation
            false,  // Not async
        );

        Ok(function_stmt)
    }
}
