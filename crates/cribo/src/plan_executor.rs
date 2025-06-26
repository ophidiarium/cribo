//! Dumb Plan Executor - Mechanical execution of BundlePlan decisions
//!
//! This module implements a stateless executor that transforms a BundlePlan
//! into a Python AST. All decisions are made during analysis phase; this
//! executor simply follows the plan without any logic or heuristics.

use anyhow::Result;
use log::{debug, trace};
use ruff_python_ast::{AtomicNodeIndex, Identifier, ModModule, Stmt, StmtImport, StmtImportFrom};
use ruff_text_size::TextRange;
use rustc_hash::FxHashMap;

use crate::{
    bundle_plan::{BundlePlan, ExecutionStep},
    cribo_graph::{CriboGraph, ItemId, ModuleId},
    orchestrator::ModuleRegistry,
};

/// Context needed for plan execution
pub struct ExecutionContext<'a> {
    pub graph: &'a CriboGraph,
    pub registry: &'a ModuleRegistry,
    pub source_asts: FxHashMap<ModuleId, ModModule>,
}

/// Execute a BundlePlan to generate the final bundled module
pub fn execute_plan(plan: &BundlePlan, context: &ExecutionContext) -> Result<ModModule> {
    debug!(
        "Starting plan execution with {} steps",
        plan.execution_plan.len()
    );

    let mut final_body = Vec::new();

    // Process execution steps in order - no decision making!
    for (idx, step) in plan.execution_plan.iter().enumerate() {
        trace!("Executing step {idx}: {step:?}");

        match execute_step(step, plan, context)? {
            Some(stmt) => final_body.push(stmt),
            None => {
                trace!("Step {idx} produced no statement");
            }
        }
    }

    debug!(
        "Plan execution complete, generated {} statements",
        final_body.len()
    );

    Ok(ModModule {
        body: final_body,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Execute a single step from the plan
/// Pure function - no state, no decisions
fn execute_step(
    step: &ExecutionStep,
    plan: &BundlePlan,
    context: &ExecutionContext,
) -> Result<Option<Stmt>> {
    match step {
        ExecutionStep::HoistFutureImport { name } => Ok(Some(generate_future_import(name))),

        ExecutionStep::HoistStdlibImport { name } => Ok(Some(generate_stdlib_import(name))),

        ExecutionStep::DefineInitFunction { module_id } => {
            // TODO: Implement wrapped module init function generation
            debug!("DefineInitFunction for module {module_id:?} - not yet implemented");
            Ok(None)
        }

        ExecutionStep::CallInitFunction {
            module_id,
            target_variable,
        } => {
            // TODO: Implement init function call generation
            debug!(
                "CallInitFunction for module {module_id:?} -> {target_variable} - not yet \
                 implemented"
            );
            Ok(None)
        }

        ExecutionStep::InlineStatement { module_id, item_id } => {
            let stmt = get_statement(&context.source_asts, *module_id, *item_id, context)?;
            let renamed_stmt = apply_ast_renames(stmt, plan, *module_id);
            Ok(Some(renamed_stmt))
        }
    }
}

/// Generate a `from __future__ import X` statement
fn generate_future_import(name: &str) -> Stmt {
    Stmt::ImportFrom(StmtImportFrom {
        module: Some(Identifier::new("__future__", TextRange::default())),
        names: vec![ruff_python_ast::Alias {
            name: Identifier::new(name, TextRange::default()),
            asname: None,
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        }],
        level: 0,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Generate a standard library import statement
fn generate_stdlib_import(name: &str) -> Stmt {
    Stmt::Import(StmtImport {
        names: vec![ruff_python_ast::Alias {
            name: Identifier::new(name, TextRange::default()),
            asname: None,
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        }],
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
fn apply_ast_renames(stmt: Stmt, _plan: &BundlePlan, _module_id: ModuleId) -> Stmt {
    // TODO: Implement AST transformation using the ast_node_renames map
    // For now, return the statement unchanged
    stmt
}

/// Initialize the plan executor by replacing the old code generator
pub fn init() -> Result<()> {
    debug!("Plan executor initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_future_import() {
        let stmt = generate_future_import("annotations");
        match stmt {
            Stmt::ImportFrom(import) => {
                assert_eq!(import.module.as_ref().unwrap().as_str(), "__future__");
                assert_eq!(import.names[0].name.as_str(), "annotations");
            }
            _ => panic!("Expected ImportFrom statement"),
        }
    }

    #[test]
    fn test_generate_stdlib_import() {
        let stmt = generate_stdlib_import("functools");
        match stmt {
            Stmt::Import(import) => {
                assert_eq!(import.names[0].name.as_str(), "functools");
            }
            _ => panic!("Expected Import statement"),
        }
    }
}
