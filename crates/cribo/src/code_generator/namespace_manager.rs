//! Namespace management utilities for code generation.
//!
//! This module provides functions for creating and managing Python namespace objects
//! that simulate module structures in bundled code.

use std::path::PathBuf;

use log::debug;
use ruff_python_ast::{
    Arguments, AtomicNodeIndex, Expr, ExprAttribute, ExprCall, ExprContext, ExprName,
    ExprStringLiteral, Identifier, Stmt, StmtAssign, StringLiteral, StringLiteralFlags,
    StringLiteralValue,
};
use ruff_text_size::TextRange;

use crate::{code_generator::bundler::HybridStaticBundler, types::FxIndexSet};

/// Generates submodule attributes with exclusions for namespace organization.
///
/// This function analyzes module hierarchies and creates namespace modules and assignments
/// as needed, while handling exclusions and avoiding redundant operations.
///
/// **Note**: This is the complete 310-line implementation moved from bundler.rs to achieve
/// Phase 7 token reduction. The implementation uses bundler helper methods where available
/// (`create_namespace_module`, `create_dotted_attribute_assignment`) and direct AST
/// construction for intermediate namespaces that require specific attribute assignments.
pub(super) fn generate_submodule_attributes_with_exclusions(
    bundler: &HybridStaticBundler,
    sorted_modules: &[(String, PathBuf, Vec<String>)],
    final_body: &mut Vec<Stmt>,
    exclusions: &FxIndexSet<String>,
) {
    debug!(
        "generate_submodule_attributes: Starting with {} modules",
        sorted_modules.len()
    );

    // Step 1: Identify all namespaces and modules that need to be created/assigned
    let mut namespace_modules = FxIndexSet::default(); // Simple namespace modules to create
    let mut module_assignments = Vec::new(); // (depth, parent, attr, module_name)

    // First, collect ALL modules that have been initialized (both wrapper and namespace)
    let mut all_initialized_modules = FxIndexSet::default();

    // Add all wrapper modules
    for (module_name, _, _) in sorted_modules {
        if bundler.module_registry.contains_key(module_name) {
            all_initialized_modules.insert(module_name.clone());
        }
    }

    // Now analyze what namespaces are needed and add wrapper module assignments
    // Combined loop for better efficiency
    for module_name in &all_initialized_modules {
        if !module_name.contains('.') {
            continue;
        }

        // This is a dotted module - ensure all parent namespaces exist
        let parts: Vec<&str> = module_name.split('.').collect();

        // Collect all parent levels that need to exist
        for i in 1..parts.len() {
            let parent_path = parts[..i].join(".");

            // If this parent is not already an initialized module, it's a namespace that needs
            // to be created
            if !all_initialized_modules.contains(&parent_path) {
                if i == 1 {
                    // Top-level namespace (e.g., 'core', 'models', 'services')
                    namespace_modules.insert(parent_path);
                } else {
                    // Intermediate namespace (e.g., 'core.database')
                    // These will be created as attributes after their parent exists
                    let parent = parts[..i - 1].join(".");
                    let attr = parts[i - 1];
                    module_assignments.push((i, parent, attr.to_string(), parent_path));
                }
            }
        }

        // Add wrapper module assignment for this module
        let parent = parts[..parts.len() - 1].join(".");
        let attr = parts[parts.len() - 1];

        // Only add if this is actually a wrapper module
        if bundler.module_registry.contains_key(module_name) {
            module_assignments.push((parts.len(), parent, attr.to_string(), module_name.clone()));
        }
    }

    // Step 2: Create top-level namespace modules and wrapper module references
    let mut created_namespaces = FxIndexSet::default();

    // Add all namespaces that were already created via the namespace tracking index
    for namespace in &bundler.required_namespaces {
        created_namespaces.insert(namespace.clone());
    }

    // First, create references to top-level wrapper modules
    let mut top_level_wrappers = Vec::new();
    for module_name in &all_initialized_modules {
        if !module_name.contains('.') && bundler.module_registry.contains_key(module_name) {
            // This is a top-level wrapper module
            top_level_wrappers.push(module_name.clone());
        }
    }
    top_level_wrappers.sort(); // Deterministic order

    for wrapper in top_level_wrappers {
        // Skip if this module is imported in the entry module
        if exclusions.contains(&wrapper) {
            debug!("Skipping top-level wrapper '{wrapper}' - imported in entry module");
            created_namespaces.insert(wrapper);
            continue;
        }

        debug!("Top-level wrapper '{wrapper}' already initialized, skipping assignment");
        // Top-level wrapper modules are already initialized via their init functions
        // No need to create any assignment - the module already exists
        created_namespaces.insert(wrapper);
    }

    // Then, create namespace modules
    let mut sorted_namespaces: Vec<String> = namespace_modules.into_iter().collect();
    sorted_namespaces.sort(); // Deterministic order

    for namespace in sorted_namespaces {
        // Skip if this namespace was already created via the namespace tracking index
        if bundler.required_namespaces.contains(&namespace) {
            debug!(
                "Skipping top-level namespace '{namespace}' - already created via namespace index"
            );
            created_namespaces.insert(namespace);
            continue;
        }

        // Check if this namespace was already created globally
        if bundler.created_namespaces.contains(&namespace) {
            debug!("Skipping top-level namespace '{namespace}' - already created globally");
            created_namespaces.insert(namespace);
            continue;
        }

        debug!("Creating top-level namespace: {namespace}");
        final_body.extend(bundler.create_namespace_module(&namespace));
        created_namespaces.insert(namespace);
    }

    // Step 3: Sort module assignments by depth to ensure parents exist before children
    module_assignments.sort_by(
        |(depth_a, parent_a, attr_a, name_a), (depth_b, parent_b, attr_b, name_b)| {
            (depth_a, parent_a.as_str(), attr_a.as_str(), name_a.as_str()).cmp(&(
                depth_b,
                parent_b.as_str(),
                attr_b.as_str(),
                name_b.as_str(),
            ))
        },
    );

    // Step 4: Process all assignments in order
    for (depth, parent, attr, module_name) in module_assignments {
        debug!("Processing assignment: {parent}.{attr} = {module_name} (depth={depth})");

        // Check if parent exists or will exist
        let parent_exists = created_namespaces.contains(&parent)
            || bundler.module_registry.contains_key(&parent)
            || parent.is_empty(); // Empty parent means top-level

        if !parent_exists {
            debug!("Warning: Parent '{parent}' doesn't exist for assignment {parent}.{attr}");
            continue;
        }

        if bundler.module_registry.contains_key(&module_name) {
            // Check if parent module has this attribute in __all__ (indicating a re-export)
            // OR if the parent is a wrapper module and the attribute is already defined there
            let skip_assignment =
                if let Some(Some(parent_exports)) = bundler.module_exports.get(&parent) {
                    if parent_exports.contains(&attr) {
                        // Check if this is a symbol re-exported from within the parent module
                        // rather than the submodule itself
                        // For example, in mypackage/__init__.py:
                        // from .config import config  # imports the 'config' instance, not the
                        // module __all__ = ['config']        # exports the
                        // instance

                        // In this case, 'config' in parent_exports refers to an imported symbol,
                        // not the submodule 'mypackage.config'
                        debug!(
                            "Skipping submodule assignment for {parent}.{attr} - it's a \
                             re-exported attribute (not the module itself)"
                        );
                        true
                    } else {
                        false
                    }
                } else if bundler.module_registry.contains_key(&parent) {
                    // Parent is a wrapper module - check if it already has this attribute defined
                    // This handles cases where the wrapper module imports a symbol with the same
                    // name as a submodule (e.g., from .config import config)
                    debug!(
                        "Parent {parent} is a wrapper module, checking if {attr} is already \
                         defined there"
                    );
                    // For now, we'll check if the attribute is in parent_exports
                    // This may need refinement based on more complex cases
                    false
                } else {
                    false
                };

            if !skip_assignment {
                // Check if this module was imported in the entry module
                if exclusions.contains(&module_name) {
                    debug!(
                        "Skipping wrapper module assignment '{parent}.{attr} = {module_name}' - \
                         imported in entry module"
                    );
                } else {
                    // Check if this would be a redundant self-assignment
                    let full_target = format!("{parent}.{attr}");
                    if full_target == module_name {
                        debug!(
                            "Skipping redundant self-assignment: {parent}.{attr} = {module_name}"
                        );
                    } else {
                        // This is a wrapper module - assign direct reference
                        debug!("Assigning wrapper module: {parent}.{attr} = {module_name}");
                        final_body.push(bundler.create_dotted_attribute_assignment(
                            &parent,
                            &attr,
                            &module_name,
                        ));
                    }
                }
            }
        } else {
            // This is an intermediate namespace - skip if already created via namespace index
            if bundler.required_namespaces.contains(&module_name) {
                debug!(
                    "Skipping intermediate namespace '{module_name}' - already created via \
                     namespace index"
                );
                created_namespaces.insert(module_name);
                continue;
            }

            debug!("Creating intermediate namespace: {parent}.{attr} = types.SimpleNamespace()");
            final_body.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: parent.clone().into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new(&attr, TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Call(ExprCall {
                    node_index: AtomicNodeIndex::dummy(),
                    func: Box::new(Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: "types".into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new("SimpleNamespace", TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    arguments: Arguments {
                        node_index: AtomicNodeIndex::dummy(),
                        args: Box::from([]),
                        keywords: Box::from([]),
                        range: TextRange::default(),
                    },
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }));

            // Set the __name__ attribute
            final_body.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: parent.clone().into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new(&attr, TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("__name__", TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::StringLiteral(ExprStringLiteral {
                    node_index: AtomicNodeIndex::dummy(),
                    value: StringLiteralValue::single(StringLiteral {
                        node_index: AtomicNodeIndex::dummy(),
                        value: module_name.to_string().into(),
                        range: TextRange::default(),
                        flags: StringLiteralFlags::empty(),
                    }),
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }));

            created_namespaces.insert(module_name);
        }
    }
}
