//! Dumb Plan Executor - Mechanical execution of BundlePlan decisions
//!
//! This module implements a stateless executor that transforms a BundlePlan
//! into a Python AST. All decisions are made during analysis phase; this
//! executor simply follows the plan without any logic or heuristics.

use anyhow::Result;
use log::{debug, trace};
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprAttribute, ExprCall, ExprName, Identifier, ModModule, Stmt,
    StmtAssign, StmtImport, StmtImportFrom, name::Name,
};
use ruff_text_size::TextRange;
use rustc_hash::FxHashMap;

use crate::{
    bundle_plan::{BundlePlan, ExecutionStep},
    cribo_graph::{CriboGraph, ItemId, ItemType, ModuleId},
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

    // First, collect namespace requirements by scanning the plan
    let namespace_requirements = collect_namespace_requirements(plan, context)?;
    debug!("Collected namespace requirements: {namespace_requirements:?}");

    let mut final_body = Vec::new();

    // Add SimpleNamespace import if needed
    if !namespace_requirements.is_empty() {
        debug!(
            "Creating SimpleNamespace import for {} namespaces",
            namespace_requirements.len()
        );
        final_body.push(generate_simple_namespace_import());
    }

    // Create namespace objects for each required module
    for (module_name, exports) in &namespace_requirements {
        debug!("Creating namespace object for module '{module_name}'");
        final_body.push(generate_namespace_object(module_name, exports));
    }

    // Process execution steps in order - no decision making!
    for (idx, step) in plan.execution_plan.iter().enumerate() {
        trace!("Executing step {idx}: {step:?}");

        match execute_step(step, plan, context, &namespace_requirements)? {
            Some(stmt) => final_body.push(stmt),
            None => {
                trace!("Step {idx} produced no statement");
            }
        }
    }

    // Populate namespace attributes after all code is inlined
    for (module_name, exports) in &namespace_requirements {
        debug!("Populating namespace attributes for module '{module_name}'");
        for export in exports {
            final_body.push(generate_namespace_attribute_assignment(module_name, export));
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
    namespace_modules: &FxHashMap<String, Vec<String>>,
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

            // Check if this is an import that should be filtered
            if should_filter_import(&stmt, context, *module_id, *item_id, namespace_modules)? {
                return Ok(None);
            }

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

/// Check if an import should be filtered (not included in the bundle)
fn should_filter_import(
    stmt: &Stmt,
    context: &ExecutionContext,
    module_id: ModuleId,
    item_id: ItemId,
    namespace_modules: &FxHashMap<String, Vec<String>>,
) -> Result<bool> {
    debug!("Checking if should filter import for module {module_id:?} item {item_id:?}");
    debug!("Statement type: {:?}", std::mem::discriminant(stmt));
    match stmt {
        Stmt::Import(import) => {
            // Check each imported name
            for alias in &import.names {
                let module_name = alias.name.as_str();

                // Check if this is a first-party module
                if context.registry.get_id_by_name(module_name).is_some() {
                    debug!("Filtering first-party import: {module_name}");
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Stmt::ImportFrom(import_from) => {
            if let Some(module) = &import_from.module {
                let module_name = module.as_str();
                debug!(
                    "ImportFrom: from {} import {:?}",
                    module_name,
                    import_from
                        .names
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>()
                );

                // Handle relative imports
                if import_from.level > 0 {
                    // Relative imports are always first-party
                    debug!(
                        "Filtering relative import: level={}, module={:?}",
                        import_from.level, module_name
                    );
                    return Ok(true);
                }

                // Check if this is a first-party module
                debug!(
                    "Checking if '{}' is first-party: {}",
                    module_name,
                    context.registry.get_id_by_name(module_name).is_some()
                );
                debug!(
                    "Also checking with __init__: {}",
                    context
                        .registry
                        .get_id_by_name(&format!("{module_name}.__init__"))
                        .is_some()
                );

                // For namespace packages, we need to check if any submodule exists
                let mut is_first_party = context.registry.get_id_by_name(module_name).is_some()
                    || context
                        .registry
                        .get_id_by_name(&format!("{module_name}.__init__"))
                        .is_some();

                // Check if any of the imported names form a valid module path
                if !is_first_party {
                    for alias in &import_from.names {
                        let full_path = format!("{module_name}.{}", alias.name.as_str());
                        if context.registry.get_id_by_name(&full_path).is_some() {
                            debug!("Found first-party module via full path: {full_path}");
                            is_first_party = true;
                            break;
                        }
                    }
                }

                if is_first_party {
                    // Check if any of the imported names will be replaced by namespace objects
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        let full_module_path = format!("{module_name}.{imported_name}");

                        debug!("Checking if '{imported_name}' will be replaced by namespace");
                        debug!(
                            "  - In namespace_modules map: {}",
                            namespace_modules.contains_key(imported_name)
                        );
                        debug!(
                            "  - Full module path '{}' in registry: {}",
                            full_module_path,
                            context.registry.get_id_by_name(&full_module_path).is_some()
                        );

                        // Check if this import is importing a module (not a symbol)
                        if context.registry.get_id_by_name(&full_module_path).is_some() {
                            debug!(
                                "Filtering module import that will be replaced by namespace: from \
                                 {module_name} import {imported_name}"
                            );
                            return Ok(true); // Filter - we'll create namespace object
                        }
                    }

                    debug!("Filtering first-party from import: {module_name}");
                    return Ok(true);
                }

                // Also check if any parent package is first-party
                let parts: Vec<&str> = module_name.split('.').collect();
                for i in 1..=parts.len() {
                    let parent = parts[..i].join(".");
                    if context.registry.get_id_by_name(&parent).is_some() {
                        debug!("Filtering first-party from import (parent package): {module_name}");
                        return Ok(true);
                    }
                }
            } else if import_from.level > 0 {
                // Relative import without module (e.g., from . import foo)
                debug!("Filtering relative import without module");
                return Ok(true);
            }

            Ok(false)
        }
        _ => Ok(false),
    }
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
            match expr {
                Expr::Name(name_expr) => {
                    let key = (self.module_id, name_expr.range);
                    if let Some(new_name) = self.renames.get(&key) {
                        trace!(
                            "Renaming identifier at {:?} from '{}' to '{}'",
                            name_expr.range, name_expr.id, new_name
                        );
                        name_expr.id = Name::new(new_name);
                    }
                }
                Expr::Attribute(attr_expr) => {
                    // For attribute access like config.DEFAULT_NAME, check if we need to rename
                    // the base object (e.g., config -> something else)
                    if let Expr::Name(base_name) = &mut *attr_expr.value {
                        let key = (self.module_id, base_name.range);
                        if let Some(new_name) = self.renames.get(&key) {
                            trace!(
                                "Renaming attribute base at {:?} from '{}' to '{}'",
                                base_name.range, base_name.id, new_name
                            );
                            base_name.id = Name::new(new_name);
                        }
                    }
                    // Continue visiting the value expression
                    self.visit_expr(&mut attr_expr.value);
                }
                _ => {}
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

/// Collect namespace requirements by analyzing filtered imports
fn collect_namespace_requirements(
    plan: &BundlePlan,
    context: &ExecutionContext,
) -> Result<FxHashMap<String, Vec<String>>> {
    let mut namespace_map = FxHashMap::default();

    // Scan through all execution steps to find filtered imports
    for step in &plan.execution_plan {
        if let ExecutionStep::InlineStatement { module_id, item_id } = step {
            // Get the statement
            let module_ast = context.source_asts.get(module_id).ok_or_else(|| {
                anyhow::anyhow!("Module {:?} not found in source ASTs", module_id)
            })?;

            let module_graph = context
                .graph
                .modules
                .get(module_id)
                .ok_or_else(|| anyhow::anyhow!("Module {:?} not found in graph", module_id))?;

            let item_data = module_graph
                .items
                .get(item_id)
                .ok_or_else(|| anyhow::anyhow!("Item {:?} not found", item_id))?;

            if let Some(stmt_index) = item_data.statement_index
                && let Some(stmt) = module_ast.body.get(stmt_index)
            {
                // Check if this is a from import that might need namespace objects
                if let Stmt::ImportFrom(import_from) = stmt
                    && let Some(module) = &import_from.module
                {
                    let module_name = module.as_str();

                    // Check if any imported names are first-party modules
                    // This handles implicit namespace packages without __init__.py
                    let mut is_first_party_import = false;
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        let full_module_path = format!("{module_name}.{imported_name}");
                        if context.registry.get_id_by_name(&full_module_path).is_some() {
                            is_first_party_import = true;
                            break;
                        }
                    }

                    if context.registry.get_id_by_name(module_name).is_some()
                        || context
                            .registry
                            .get_id_by_name(&format!("{module_name}.__init__"))
                            .is_some()
                        || is_first_party_import
                    {
                        // This is a first-party import, check what's being imported
                        debug!("Checking first-party import from {module_name}");
                        for alias in &import_from.names {
                            let imported_name = alias.name.as_str();
                            let full_module_path = format!("{module_name}.{imported_name}");
                            debug!(
                                "Checking if '{imported_name}' is a module (full path: \
                                 {full_module_path})"
                            );

                            // Check if this looks like a module import (not a symbol)
                            if let Some(target_module_id) =
                                context.registry.get_id_by_name(&full_module_path)
                            {
                                // This is importing a submodule - we need a namespace
                                // object
                                debug!(
                                    "Found module import: from {module_name} import \
                                     {imported_name} (module_id: {target_module_id:?})"
                                );
                                // Get the exports for this module
                                let exports = collect_module_exports(context, target_module_id)?;
                                debug!("Module '{imported_name}' exports: {exports:?}");
                                namespace_map.insert(imported_name.to_string(), exports);
                            } else {
                                debug!(
                                    "'{imported_name}' is not a module (full path \
                                     '{full_module_path}' not found in registry)"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(namespace_map)
}

/// Collect all exports from a module
fn collect_module_exports(context: &ExecutionContext, module_id: ModuleId) -> Result<Vec<String>> {
    let mut exports = Vec::new();

    if let Some(module_graph) = context.graph.modules.get(&module_id) {
        for item_data in module_graph.items.values() {
            match &item_data.item_type {
                ItemType::FunctionDef { name } | ItemType::ClassDef { name } => {
                    exports.push(name.clone());
                }
                ItemType::Assignment { targets } => {
                    for target in targets {
                        if !target.starts_with('_') {
                            exports.push(target.clone());
                        }
                    }
                }
                _ => {}
            }

            // Also check defined_symbols
            for symbol in &item_data.defined_symbols {
                if !symbol.starts_with('_') && !exports.contains(symbol) {
                    exports.push(symbol.clone());
                }
            }
        }
    }

    Ok(exports)
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
fn generate_namespace_object(module_name: &str, _exports: &[String]) -> Stmt {
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

/// Generate namespace attribute assignment: namespace.attr = value
fn generate_namespace_attribute_assignment(namespace_name: &str, attribute_name: &str) -> Stmt {
    // Create namespace.attribute target
    let target = Expr::Attribute(ExprAttribute {
        value: Box::new(Expr::Name(ExprName {
            id: Name::new(namespace_name),
            ctx: ruff_python_ast::ExprContext::Load,
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        })),
        attr: Identifier::new(attribute_name, TextRange::default()),
        ctx: ruff_python_ast::ExprContext::Store,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    });

    // Create value (just the attribute name as a variable reference)
    let value = Expr::Name(ExprName {
        id: Name::new(attribute_name),
        ctx: ruff_python_ast::ExprContext::Load,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    });

    // Create assignment: namespace.attribute = attribute
    Stmt::Assign(StmtAssign {
        targets: vec![target],
        value: Box::new(value),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
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
