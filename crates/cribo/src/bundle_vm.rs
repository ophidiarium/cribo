//! Bundle Virtual Machine - Mechanical execution of bundling instructions
//!
//! This module implements a truly "dumb" VM that mechanically executes a BundleProgram
//! to produce a Python AST. The VM performs only two primitive operations:
//! 1. Insert pre-built statements
//! 2. Copy statements from source with rename transformations

use anyhow::Result;
use log::{debug, trace};
use ruff_python_ast::{
    AtomicNodeIndex, Expr, Identifier, ModModule, Stmt,
    name::Name,
    visitor::transformer::{Transformer, walk_expr, walk_stmt},
};
use ruff_text_size::TextRange;
use rustc_hash::FxHashMap;

use crate::{
    bundle_compiler::{BundleProgram, ExecutionStep},
    cribo_graph::{CriboGraph, ItemId, ModuleId},
    module_registry::ModuleRegistry,
};

/// Context needed for plan execution
pub struct ExecutionContext<'a> {
    pub graph: &'a CriboGraph,
    pub registry: &'a ModuleRegistry,
    pub source_asts: FxHashMap<ModuleId, ModModule>,
}

/// Run the bundle VM to generate the final bundled module
/// This is a truly dumb VM - it just mechanically executes instructions
pub fn run(program: &BundleProgram, context: &ExecutionContext) -> Result<ModModule> {
    debug!(
        "Starting bundle VM execution with {} instructions",
        program.steps.len()
    );

    let mut final_body = Vec::new();

    // Process each instruction mechanically - no analysis, no decisions!
    for (idx, step) in program.steps.iter().enumerate() {
        trace!("Executing instruction {idx}: {step:?}");

        match step {
            ExecutionStep::InsertStatement { stmt } => {
                // Simply insert the pre-built statement
                final_body.push(stmt.clone());
            }

            ExecutionStep::CopyStatement {
                source_module,
                item_id,
            } => {
                // Get the original statement
                let stmt = get_statement(&context.source_asts, *source_module, *item_id, context)?;

                // Apply renames (the only transformation we perform)
                let renamed_stmt =
                    apply_ast_renames(stmt, &program.ast_node_renames, *source_module);

                final_body.push(renamed_stmt);
            }
        }
    }

    debug!(
        "Bundle VM execution complete, generated {} statements",
        final_body.len()
    );

    Ok(ModModule {
        body: final_body,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Get a statement from the source ASTs
fn get_statement(
    source_asts: &FxHashMap<ModuleId, ModModule>,
    module_id: ModuleId,
    item_id: ItemId,
    context: &ExecutionContext,
) -> Result<Stmt> {
    let module_ast = source_asts
        .get(&module_id)
        .ok_or_else(|| anyhow::anyhow!("Module {:?} not found in source ASTs", module_id))?;

    // Get the module graph to find the item's statement index
    let module_graph = context
        .graph
        .modules
        .get(&module_id)
        .ok_or_else(|| anyhow::anyhow!("Module {:?} not found in graph", module_id))?;

    let item_data = module_graph
        .items
        .get(&item_id)
        .ok_or_else(|| anyhow::anyhow!("Item {:?} not found in module {:?}", item_id, module_id))?;

    let stmt_index = item_data
        .statement_index
        .ok_or_else(|| anyhow::anyhow!("Item {:?} has no statement index", item_id))?;

    debug!(
        "Getting statement at index {} for item {:?} of type {:?} in module {:?}",
        stmt_index, item_id, item_data.item_type, module_id
    );

    module_ast.body.get(stmt_index).cloned().ok_or_else(|| {
        anyhow::anyhow!(
            "Statement index {} out of bounds for module {:?} (has {} statements)",
            stmt_index,
            module_id,
            module_ast.body.len()
        )
    })
}

/// Apply AST node renames to a statement
fn apply_ast_renames(
    mut stmt: Stmt,
    ast_node_renames: &FxHashMap<(ModuleId, TextRange), String>,
    module_id: ModuleId,
) -> Stmt {
    struct RenameTransformer<'a> {
        renames: &'a FxHashMap<(ModuleId, TextRange), String>,
        module_id: ModuleId,
    }

    impl<'a> Transformer for RenameTransformer<'a> {
        fn visit_expr(&self, expr: &mut Expr) {
            if let Expr::Name(name_expr) = expr {
                let key = (self.module_id, name_expr.range);
                if let Some(new_name) = self.renames.get(&key) {
                    trace!(
                        "Renaming identifier at {:?} from '{}' to '{}'",
                        name_expr.range, name_expr.id, new_name
                    );
                    name_expr.id = Name::new(new_name);
                }
            }

            // Continue visiting child expressions
            walk_expr(self, expr);
        }

        fn visit_stmt(&self, stmt: &mut Stmt) {
            // Handle class and function definitions
            match stmt {
                Stmt::ClassDef(class_def) => {
                    let key = (self.module_id, class_def.name.range);
                    if let Some(new_name) = self.renames.get(&key) {
                        trace!(
                            "Renaming class '{}' at {:?} to '{}'",
                            class_def.name, class_def.name.range, new_name
                        );
                        class_def.name = Identifier::new(new_name, class_def.name.range);
                    }
                }
                Stmt::FunctionDef(func_def) => {
                    let key = (self.module_id, func_def.name.range);
                    if let Some(new_name) = self.renames.get(&key) {
                        trace!(
                            "Renaming function '{}' at {:?} to '{}'",
                            func_def.name, func_def.name.range, new_name
                        );
                        func_def.name = Identifier::new(new_name, func_def.name.range);
                    }
                }
                _ => {}
            }

            // Continue visiting child statements
            walk_stmt(self, stmt);
        }
    }

    let transformer = RenameTransformer {
        renames: ast_node_renames,
        module_id,
    };

    transformer.visit_stmt(&mut stmt);
    stmt
}

/// Initialize the bundle VM
pub fn init() -> Result<()> {
    debug!("Bundle VM initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_builder;

    #[test]
    fn test_simple_execution() {
        // Create a simple program with just insert instructions
        let program = BundleProgram {
            steps: vec![
                ExecutionStep::InsertStatement {
                    stmt: ast_builder::import("os"),
                },
                ExecutionStep::InsertStatement {
                    stmt: ast_builder::assign("x", ast_builder::name("y")),
                },
            ],
            ast_node_renames: FxHashMap::default(),
        };

        // Create empty context
        let graph = CriboGraph::default();
        let registry = ModuleRegistry::new();
        let context = ExecutionContext {
            graph: &graph,
            registry: &registry,
            source_asts: FxHashMap::default(),
        };

        // Run the VM
        let result = run(&program, &context).unwrap();

        // Verify we got two statements
        assert_eq!(result.body.len(), 2);

        // Verify the first is an import
        assert!(matches!(result.body[0], Stmt::Import(_)));

        // Verify the second is an assignment
        assert!(matches!(result.body[1], Stmt::Assign(_)));
    }
}
