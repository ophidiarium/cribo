//! Finalization phase for init function transformation
//!
//! This phase builds the final function statement from the accumulated state.

#![allow(unused_imports)] // Will be used when orchestrator calls this phase

use ruff_python_ast::{AtomicNodeIndex, ExprContext, Identifier, ModModule};
use ruff_text_size::TextRange;

use super::{TransformError, state::InitFunctionState};
use crate::{
    ast_builder,
    code_generator::{bundler::Bundler, context::ModuleTransformContext},
};

/// Module object parameter name used in generated init functions
const SELF_PARAM: &str = "self";

/// Phase responsible for finalizing and building the init function statement
#[allow(dead_code)] // Will be used when orchestrator is created
pub struct FinalizationPhase;

impl FinalizationPhase {
    /// Build the final function statement from accumulated state
    ///
    /// This phase:
    /// 1. Marks the module as fully initialized (__initialized__ = True)
    /// 2. Clears the initializing flag (__initializing__ = False)
    /// 3. Returns the module object (return self)
    /// 4. Creates function parameters with 'self' parameter
    /// 5. Builds and returns the complete function definition
    ///
    /// Note: This phase consumes the state (takes ownership) as it's the final phase
    #[allow(dead_code)] // Will be called by orchestrator
    pub fn build_function_stmt(
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        mut state: InitFunctionState,
    ) -> Result<ruff_python_ast::Stmt, TransformError> {
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
                module_name: ctx.module_name.to_string(),
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
