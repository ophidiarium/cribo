//! Wrapper globals collection phase for init function transformation
//!
//! This phase collects wrapper module namespace variables that need global declarations
//! by traversing the processed body using a visitor pattern.

use log::debug;
use ruff_python_ast::{
    Expr, Stmt,
    visitor::source_order::{self, SourceOrderVisitor},
};

use super::state::InitFunctionState;
use crate::{ast_builder, types::FxIndexSet};

/// Phase responsible for collecting and declaring wrapper module globals
pub(crate) struct WrapperGlobalsPhase;

impl WrapperGlobalsPhase {
    /// Collect and declare wrapper module namespace variables
    ///
    /// This phase:
    /// 1. Uses a visitor to traverse the processed body
    /// 2. Identifies assignments where the target is passed as an argument to init functions
    /// 3. Collects these variables as needing global declarations
    /// 4. Adds sorted global declarations to the body
    ///
    /// This is necessary because wrapper module init functions may pass their own namespace
    /// object as an argument (e.g., `foo = init_foo(foo)`), which requires `foo` to be
    /// declared as global to avoid `UnboundLocalError`.
    pub(crate) fn execute(processed_body: &[Stmt], state: &mut InitFunctionState) {
        // Use visitor to properly traverse the AST
        let wrapper_globals_needed = WrapperGlobalCollector::collect(processed_body);

        // Add global declarations for wrapper module namespace variables at the beginning
        if !wrapper_globals_needed.is_empty() {
            let mut globals: Vec<&str> =
                wrapper_globals_needed.iter().map(String::as_str).collect();
            globals.sort_unstable();

            debug!(
                "Adding global declarations for {} wrapper module namespace variables: {:?}",
                globals.len(),
                globals
            );

            state.body.push(ast_builder::statements::global(globals));
        }
    }
}

/// Visitor for collecting wrapper module globals
struct WrapperGlobalCollector {
    globals_needed: FxIndexSet<String>,
}

impl WrapperGlobalCollector {
    fn new() -> Self {
        Self {
            globals_needed: FxIndexSet::default(),
        }
    }

    fn collect(processed_body: &[Stmt]) -> FxIndexSet<String> {
        let mut collector = Self::new();
        source_order::walk_body(&mut collector, processed_body);
        collector.globals_needed
    }
}

impl<'a> SourceOrderVisitor<'a> for WrapperGlobalCollector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        // Check if this assignment needs a global declaration
        if let Stmt::Assign(assign) = stmt {
            // Check if the value is a call to an init function
            if let Expr::Call(call) = assign.value.as_ref()
                && let Expr::Name(name) = call.func.as_ref()
                && crate::code_generator::module_registry::is_init_function(name.id.as_str())
            {
                // Check if the assignment target is also used as an argument
                if assign.targets.len() == 1
                    && let Expr::Name(target) = &assign.targets[0]
                {
                    // Check if the target is also passed as an argument
                    let needs_global = call.arguments.args.iter().any(|arg| {
                        matches!(arg, Expr::Name(arg_name) if arg_name.id.as_str() == target.id.as_str())
                    });
                    if needs_global {
                        debug!(
                            "Wrapper module namespace variable '{}' needs global declaration",
                            target.id
                        );
                        self.globals_needed.insert(target.id.to_string());
                    }
                }
            }
        }

        // Continue traversal to children
        source_order::walk_stmt(self, stmt);
    }
}
