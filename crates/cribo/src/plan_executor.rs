//! Dumb Plan Executor - Mechanical execution of BundlePlan decisions
//!
//! This module implements a stateless executor that transforms a BundlePlan
//! into a Python AST. All decisions are made during analysis phase; this
//! executor simply follows the plan without any logic or heuristics.

use anyhow::Result;
use log::{debug, trace};
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprAttribute, ExprCall, ExprName, Identifier, ModModule, Stmt,
    StmtAssign, StmtFunctionDef, StmtImport, StmtImportFrom, name::Name,
};
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
/// This is a truly dumb executor - it just mechanically executes steps
pub fn execute_plan(plan: &BundlePlan, context: &ExecutionContext) -> Result<ModModule> {
    debug!(
        "Starting dumb plan execution with {} steps",
        plan.execution_plan.len()
    );

    let mut final_body = Vec::new();
    let mut namespace_objects = FxHashMap::default();

    // Process each step mechanically - no analysis, no decisions!
    for (idx, step) in plan.execution_plan.iter().enumerate() {
        trace!("Executing step {idx}: {step:?}");

        match step {
            ExecutionStep::HoistFutureImport { name } => {
                final_body.push(generate_future_import(name));
            }

            ExecutionStep::HoistStdlibImport { name } => {
                final_body.push(generate_stdlib_import(name));
            }

            ExecutionStep::CreateModuleNamespace { target_name } => {
                // First, ensure we have SimpleNamespace imported
                if namespace_objects.is_empty() {
                    final_body.push(generate_simple_namespace_import());
                }

                // Create the namespace object
                final_body.push(generate_namespace_object(target_name));
                namespace_objects.insert(target_name.clone(), Vec::<String>::new());
            }

            ExecutionStep::CopyStatementToNamespace {
                from_module,
                item_id,
                target_object,
                target_attribute,
            } => {
                // Get the statement from source
                let stmt = get_statement(&context.source_asts, *from_module, *item_id, context)?;

                // Transform it to namespace.attr = value
                if let Some(assignment) =
                    transform_to_namespace_assignment(stmt, target_object, target_attribute)
                {
                    final_body.push(assignment);
                }
            }

            ExecutionStep::AddImport { module_name, alias } => {
                final_body.push(generate_import(module_name, alias.as_deref()));
            }

            ExecutionStep::AddFromImport {
                module_name,
                symbols,
                level,
            } => {
                final_body.push(generate_from_import(module_name, symbols, *level));
            }

            ExecutionStep::DefineInitFunction { module_id } => {
                // Get the module's statements
                let module_ast = context.source_asts.get(module_id).ok_or_else(|| {
                    anyhow::anyhow!("Module {:?} not found in source ASTs", module_id)
                })?;

                // Wrap them in an init function
                let init_function = generate_init_function(module_id, &module_ast.body);
                final_body.push(init_function);
            }

            ExecutionStep::CallInitFunction {
                module_id,
                target_variable,
            } => {
                final_body.push(generate_init_call(module_id, target_variable));
            }

            ExecutionStep::InlineStatement { module_id, item_id } => {
                let stmt = get_statement(&context.source_asts, *module_id, *item_id, context)?;

                // Apply AST renames
                let renamed_stmt = apply_ast_renames(stmt, plan, *module_id);
                final_body.push(renamed_stmt);
            }
        }
    }

    debug!(
        "Dumb plan execution complete, generated {} statements",
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

    module_ast.body.get(stmt_index).cloned().ok_or_else(|| {
        anyhow::anyhow!(
            "Statement index {} out of bounds for module {:?} (has {} statements)",
            stmt_index,
            module_id,
            module_ast.body.len()
        )
    })
}

/// Transform a statement to a namespace attribute assignment
fn transform_to_namespace_assignment(
    stmt: Stmt,
    target_object: &str,
    target_attribute: &str,
) -> Option<Stmt> {
    // For assignments like `x = 5`, transform to `namespace.x = 5`
    match stmt {
        Stmt::Assign(mut assign) => {
            // Create namespace.attribute as target
            let namespace_attr = Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: Name::new(target_object),
                    ctx: ruff_python_ast::ExprContext::Load,
                    range: TextRange::default(),
                    node_index: AtomicNodeIndex::dummy(),
                })),
                attr: Identifier::new(target_attribute, TextRange::default()),
                ctx: ruff_python_ast::ExprContext::Store,
                range: TextRange::default(),
                node_index: AtomicNodeIndex::dummy(),
            });

            assign.targets = vec![namespace_attr];
            Some(Stmt::Assign(assign))
        }
        _ => None,
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

/// Generate an import statement with optional alias
fn generate_import(module_name: &str, alias: Option<&str>) -> Stmt {
    Stmt::Import(StmtImport {
        names: vec![ruff_python_ast::Alias {
            name: Identifier::new(module_name, TextRange::default()),
            asname: alias.map(|a| Identifier::new(a, TextRange::default())),
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        }],
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Generate a from import statement
fn generate_from_import(
    module_name: &str,
    symbols: &[(String, Option<String>)],
    level: u32,
) -> Stmt {
    let names = symbols
        .iter()
        .map(|(name, alias)| ruff_python_ast::Alias {
            name: Identifier::new(name, TextRange::default()),
            asname: alias
                .as_ref()
                .map(|a| Identifier::new(a, TextRange::default())),
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        })
        .collect();

    Stmt::ImportFrom(StmtImportFrom {
        module: if level == 0 || !module_name.is_empty() {
            Some(Identifier::new(module_name, TextRange::default()))
        } else {
            None
        },
        names,
        level,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Generate `from types import SimpleNamespace` import
fn generate_simple_namespace_import() -> Stmt {
    Stmt::ImportFrom(StmtImportFrom {
        module: Some(Identifier::new("types", TextRange::default())),
        names: vec![ruff_python_ast::Alias {
            name: Identifier::new("SimpleNamespace", TextRange::default()),
            asname: None,
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        }],
        level: 0,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Generate a namespace object assignment
fn generate_namespace_object(module_name: &str) -> Stmt {
    // Create SimpleNamespace() call
    let namespace_call = Expr::Call(ExprCall {
        func: Box::new(Expr::Name(ExprName {
            id: Name::new_static("SimpleNamespace"),
            ctx: ruff_python_ast::ExprContext::Load,
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        })),
        arguments: ruff_python_ast::Arguments {
            args: Box::new([]),
            keywords: Box::new([]),
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        },
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    });

    // Create assignment: module_name = SimpleNamespace()
    Stmt::Assign(StmtAssign {
        targets: vec![Expr::Name(ExprName {
            id: Name::new(module_name),
            ctx: ruff_python_ast::ExprContext::Store,
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        })],
        value: Box::new(namespace_call),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Generate an init function that wraps module statements
fn generate_init_function(module_id: &ModuleId, statements: &[Stmt]) -> Stmt {
    let function_name = format!("__cribo_init_{module_id:?}");

    Stmt::FunctionDef(StmtFunctionDef {
        name: Identifier::new(&function_name, TextRange::default()),
        type_params: None,
        parameters: Box::new(ruff_python_ast::Parameters {
            posonlyargs: vec![],
            args: vec![],
            vararg: None,
            kwonlyargs: vec![],
            kwarg: None,
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        }),
        body: statements.to_vec(),
        decorator_list: vec![],
        returns: None,
        is_async: false,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Generate a call to an init function
fn generate_init_call(module_id: &ModuleId, target_variable: &str) -> Stmt {
    let function_name = format!("__cribo_init_{module_id:?}");

    let call_expr = Expr::Call(ExprCall {
        func: Box::new(Expr::Name(ExprName {
            id: Name::new(&function_name),
            ctx: ruff_python_ast::ExprContext::Load,
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        })),
        arguments: ruff_python_ast::Arguments {
            args: Box::new([]),
            keywords: Box::new([]),
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        },
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    });

    Stmt::Assign(StmtAssign {
        targets: vec![Expr::Name(ExprName {
            id: Name::new(target_variable),
            ctx: ruff_python_ast::ExprContext::Store,
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        })],
        value: Box::new(call_expr),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Apply AST node renames to a statement
fn apply_ast_renames(mut stmt: Stmt, plan: &BundlePlan, module_id: ModuleId) -> Stmt {
    use ruff_python_ast::visitor::transformer::{Transformer, walk_expr, walk_stmt};

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
        renames: &plan.ast_node_renames,
        module_id,
    };

    transformer.visit_stmt(&mut stmt);
    stmt
}

/// Initialize the plan executor
pub fn init() -> Result<()> {
    debug!("Dumb plan executor initialized");
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
                assert_eq!(
                    import
                        .module
                        .as_ref()
                        .expect("Future import should have a module")
                        .as_str(),
                    "__future__"
                );
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
