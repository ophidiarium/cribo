//! Module registry management for code bundling
//!
//! This module handles:
//! - Module cache initialization and population
//! - Module naming and identifier generation
//! - Module attribute assignments
//! - Registry and sys.modules synchronization

use std::path::PathBuf;

use log::debug;
use ruff_python_ast::{
    Alias, AtomicNodeIndex, Expr, ExprAttribute, ExprCall, ExprContext, ExprDict, ExprName,
    ExprStringLiteral, ExprSubscript, Identifier, ModModule, Stmt, StmtAssign, StmtExpr,
    StmtImport, StmtImportFrom, StmtPass, StringLiteral, StringLiteralFlags, StringLiteralValue,
};
use ruff_text_size::TextRange;

use crate::types::{FxIndexMap, FxIndexSet};

/// Generate module cache initialization
pub fn generate_module_cache_init() -> Stmt {
    // __cribo_module_cache__ = {}
    let assign = StmtAssign {
        node_index: AtomicNodeIndex::dummy(),
        targets: vec![Expr::Name(ExprName {
            node_index: AtomicNodeIndex::dummy(),
            id: "__cribo_module_cache__".into(),
            ctx: ExprContext::Store,
            range: TextRange::default(),
        })],
        value: Box::new(Expr::Dict(ExprDict {
            node_index: AtomicNodeIndex::dummy(),
            items: vec![],
            range: TextRange::default(),
        })),
        range: TextRange::default(),
    };

    Stmt::Assign(assign)
}

/// Generate module cache population
pub fn generate_module_cache_population(
    modules: &[(String, ModModule, PathBuf, String)],
) -> Vec<Stmt> {
    let mut stmts = Vec::new();

    // For each module, add: __cribo_module_cache__["module.name"] = _ModuleNamespace()
    for (module_name, _, _, _) in modules {
        let assign = StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Subscript(ExprSubscript {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "__cribo_module_cache__".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                slice: Box::new(Expr::StringLiteral(ExprStringLiteral {
                    node_index: AtomicNodeIndex::dummy(),
                    value: StringLiteralValue::single(StringLiteral {
                        node_index: AtomicNodeIndex::dummy(),
                        value: module_name.clone().into_boxed_str(),
                        flags: StringLiteralFlags::empty(),
                        range: TextRange::default(),
                    }),
                    range: TextRange::default(),
                })),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "_ModuleNamespace".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: ruff_python_ast::Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        };
        stmts.push(Stmt::Assign(assign));
    }

    stmts
}

/// Generate sys.modules sync
pub fn generate_sys_modules_sync() -> Vec<Stmt> {
    let mut stmts = Vec::new();

    // import sys
    stmts.push(Stmt::Import(StmtImport {
        node_index: AtomicNodeIndex::dummy(),
        names: vec![Alias {
            node_index: AtomicNodeIndex::dummy(),
            name: Identifier::new("sys", TextRange::default()),
            asname: None,
            range: TextRange::default(),
        }],
        range: TextRange::default(),
    }));

    // sys.modules.update(__cribo_module_cache__)
    let update_call = Stmt::Expr(StmtExpr {
        node_index: AtomicNodeIndex::dummy(),
        value: Box::new(Expr::Call(ExprCall {
            node_index: AtomicNodeIndex::dummy(),
            func: Box::new(Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "sys".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("modules", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("update", TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: ruff_python_ast::Arguments {
                node_index: AtomicNodeIndex::dummy(),
                args: Box::from([Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "__cribo_module_cache__".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })]),
                keywords: Box::from([]),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        })),
        range: TextRange::default(),
    });
    stmts.push(update_call);

    stmts
}

/// Generate registries and hook
pub fn generate_registries_and_hook() -> Vec<Stmt> {
    // No longer needed - we don't use sys.modules or import hooks
    Vec::new()
}

/// Generate module init call
pub fn generate_module_init_call(
    _synthetic_name: &str,
    module_name: &str,
    init_func_name: Option<&str>,
    module_registry: &FxIndexMap<String, String>,
    generate_merge_module_attributes: impl Fn(&mut Vec<Stmt>, &str, &str),
) -> Vec<Stmt> {
    let mut statements = Vec::new();

    if let Some(init_func_name) = init_func_name {
        // Check if this module is a parent namespace that already exists
        // This happens when a module like 'services.auth' has both:
        // 1. Its own __init__.py (wrapper module)
        // 2. Submodules like 'services.auth.manager'
        let is_parent_namespace = module_registry
            .iter()
            .any(|(name, _)| name != module_name && name.starts_with(&format!("{module_name}.")));

        if is_parent_namespace {
            // For parent namespaces, we need to merge attributes instead of overwriting
            // Generate code that calls the init function and merges its attributes
            debug!("Module '{module_name}' is a parent namespace - generating merge code");

            // First, create a variable to hold the init result
            let init_result_var = "__cribo_init_result";
            statements.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: init_result_var.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Call(ExprCall {
                    node_index: AtomicNodeIndex::dummy(),
                    func: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: init_func_name.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    arguments: ruff_python_ast::Arguments {
                        node_index: AtomicNodeIndex::dummy(),
                        args: Box::from([]),
                        keywords: Box::from([]),
                        range: TextRange::default(),
                    },
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }));

            // Generate the merge attributes code
            generate_merge_module_attributes(&mut statements, module_name, init_result_var);

            // Assign the init result to the module variable
            statements.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: module_name.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: init_result_var.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }));
        } else {
            // Direct assignment for modules that aren't parent namespaces
            let target_expr = if module_name.contains('.') {
                // For dotted modules like models.base, create an attribute expression
                let parts: Vec<&str> = module_name.split('.').collect();
                let mut expr = Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: parts[0].into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                });

                for (i, part) in parts[1..].iter().enumerate() {
                    let ctx = if i == parts.len() - 2 {
                        ExprContext::Store // Last part is Store context
                    } else {
                        ExprContext::Load
                    };
                    expr = Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(expr),
                        attr: Identifier::new(*part, TextRange::default()),
                        ctx,
                        range: TextRange::default(),
                    });
                }
                expr
            } else {
                // For simple modules, use direct name
                Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: module_name.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })
            };

            // Generate: module_name = __cribo_init_synthetic_name()
            // or: parent.child = __cribo_init_synthetic_name()
            statements.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![target_expr],
                value: Box::new(Expr::Call(ExprCall {
                    node_index: AtomicNodeIndex::dummy(),
                    func: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: init_func_name.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    arguments: ruff_python_ast::Arguments {
                        node_index: AtomicNodeIndex::dummy(),
                        args: Box::from([]),
                        keywords: Box::from([]),
                        range: TextRange::default(),
                    },
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }));
        }
    } else {
        statements.push(Stmt::Pass(StmtPass {
            node_index: AtomicNodeIndex::dummy(),
            range: TextRange::default(),
        }));
    }

    statements
}

/// Get synthetic module name
pub fn get_synthetic_module_name(module_name: &str, content_hash: &str) -> String {
    let module_name_escaped = sanitize_module_name_for_identifier(module_name);
    // Use first 6 characters of content hash for readability
    let short_hash = &content_hash[..6];
    format!("__cribo_{short_hash}_{module_name_escaped}")
}

/// Sanitize a module name for use in a Python identifier
/// This is a simple character replacement - collision handling should be done by the caller
pub fn sanitize_module_name_for_identifier(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            // Replace common invalid characters with descriptive names
            '-' => '_',
            '.' => '_',
            ' ' => '_',
            // For other non-alphanumeric characters, replace with underscore
            c if c.is_alphanumeric() || c == '_' => c,
            _ => '_',
        })
        .collect::<String>()
}

/// Generate a unique symbol name to avoid conflicts
pub fn generate_unique_name(base_name: &str, existing_symbols: &FxIndexSet<String>) -> String {
    if !existing_symbols.contains(base_name) {
        return base_name.to_string();
    }

    // Try adding numeric suffixes
    for i in 1..1000 {
        let candidate = format!("{base_name}_{i}");
        if !existing_symbols.contains(&candidate) {
            return candidate;
        }
    }

    // Fallback with module prefix
    format!("__cribo_renamed_{base_name}")
}

/// Check if a local name conflicts with any symbol in the module
pub fn check_local_name_conflict(ast: &ModModule, name: &str) -> bool {
    for stmt in &ast.body {
        match stmt {
            Stmt::ClassDef(class_def) => {
                if class_def.name.as_str() == name {
                    return true;
                }
            }
            Stmt::FunctionDef(func_def) => {
                if func_def.name.as_str() == name {
                    return true;
                }
            }
            Stmt::Assign(assign_stmt) => {
                for target in &assign_stmt.targets {
                    if let Expr::Name(name_expr) = target
                        && name_expr.id.as_str() == name
                    {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Create a module attribute assignment statement
pub fn create_module_attr_assignment(module_var: &str, attr_name: &str) -> Stmt {
    Stmt::Assign(StmtAssign {
        node_index: AtomicNodeIndex::dummy(),
        targets: vec![Expr::Attribute(ExprAttribute {
            node_index: AtomicNodeIndex::dummy(),
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: module_var.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            attr: Identifier::new(attr_name, TextRange::default()),
            ctx: ExprContext::Store,
            range: TextRange::default(),
        })],
        value: Box::new(Expr::Name(ExprName {
            node_index: AtomicNodeIndex::dummy(),
            id: attr_name.into(),
            ctx: ExprContext::Load,
            range: TextRange::default(),
        })),
        range: TextRange::default(),
    })
}

/// Create a reassignment statement (original_name = renamed_name)
pub fn create_reassignment(original_name: &str, renamed_name: &str) -> Stmt {
    Stmt::Assign(StmtAssign {
        node_index: AtomicNodeIndex::dummy(),
        targets: vec![Expr::Name(ExprName {
            node_index: AtomicNodeIndex::dummy(),
            id: original_name.into(),
            ctx: ExprContext::Store,
            range: TextRange::default(),
        })],
        value: Box::new(Expr::Name(ExprName {
            node_index: AtomicNodeIndex::dummy(),
            id: renamed_name.into(),
            ctx: ExprContext::Load,
            range: TextRange::default(),
        })),
        range: TextRange::default(),
    })
}

/// Create assignments for inlined imports
#[allow(clippy::too_many_arguments)]
pub fn create_assignments_for_inlined_imports(
    import_from: StmtImportFrom,
    module_name: &str,
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    module_registry: &FxIndexMap<String, String>,
    inlined_modules: &FxIndexSet<String>,
    bundled_modules: &FxIndexSet<String>,
    create_namespace_with_name: impl Fn(&str, &str) -> Vec<Stmt>,
) -> Vec<Stmt> {
    let mut assignments = Vec::new();

    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        let local_name = alias.asname.as_ref().unwrap_or(&alias.name);

        // Check if we're importing a module itself (not a symbol from it)
        // This happens when the imported name refers to a submodule
        let full_module_path = format!("{module_name}.{imported_name}");

        // Check if this is a module import
        // First check if it's a wrapped module
        if module_registry.contains_key(&full_module_path) {
            // For pure static approach, we don't use sys.modules
            // Instead, we'll handle this as a deferred import
            log::debug!("Module '{full_module_path}' is a wrapped module, deferring import");
            // Skip this - it will be handled differently
            continue;
        } else if inlined_modules.contains(&full_module_path)
            || bundled_modules.contains(&full_module_path)
        {
            // Create a namespace object for the inlined module
            log::debug!(
                "Creating namespace object for module '{imported_name}' imported from \
                 '{module_name}' - module was inlined"
            );

            // Create a SimpleNamespace-like object with __name__ set
            let namespace_stmts =
                create_namespace_with_name(local_name.as_str(), &full_module_path);
            assignments.extend(namespace_stmts);

            // Now add all symbols from the inlined module to the namespace
            // This should come from semantic analysis of what symbols the module exports
            if let Some(module_renames) = symbol_renames.get(&full_module_path) {
                // Add each symbol from the module to the namespace
                for (original_name, renamed_name) in module_renames {
                    // base.original_name = renamed_name
                    assignments.push(Stmt::Assign(StmtAssign {
                        node_index: AtomicNodeIndex::dummy(),
                        targets: vec![Expr::Attribute(ExprAttribute {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: local_name.as_str().into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            attr: Identifier::new(original_name, TextRange::default()),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: renamed_name.clone().into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        range: TextRange::default(),
                    }));
                }
            }
        } else {
            // Regular symbol import
            // Check if this symbol was renamed during inlining
            let actual_name = if let Some(module_renames) = symbol_renames.get(module_name) {
                module_renames
                    .get(imported_name)
                    .map(|s| s.as_str())
                    .unwrap_or(imported_name)
            } else {
                imported_name
            };

            // Only create assignment if the names are different
            if local_name.as_str() != actual_name {
                log::debug!(
                    "Creating assignment: {local_name} = {actual_name} (from inlined module \
                     '{module_name}')"
                );

                let assignment = StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: local_name.as_str().into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: actual_name.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                };
                assignments.push(Stmt::Assign(assignment));
            }
        }
    }

    assignments
}
