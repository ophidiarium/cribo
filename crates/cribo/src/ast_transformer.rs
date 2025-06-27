//! AST Transformer for applying transformations to AST nodes
//!
//! This module implements the logic to apply transformation metadata to AST nodes
//! and render them to code. It's responsible for executing the transformation plan
//! produced by the analysis phase.

use log::{debug, trace};
use ruff_python_ast::{
    Expr, ExprName, HasNodeIndex, NodeIndex, Stmt, visitor::transformer::Transformer,
};
use rustc_hash::FxHashMap;

use crate::{
    cribo_graph::ModuleId,
    transformations::{ImportData, TransformationMetadata},
};

/// AST Transformer that applies transformations and renders to code
pub struct AstTransformer<'a> {
    /// Map of transformations indexed by NodeIndex
    transformations: &'a FxHashMap<NodeIndex, Vec<TransformationMetadata>>,
    /// Current module being transformed
    current_module: ModuleId,
    /// Stylist for code generation
    stylist: &'a ruff_python_codegen::Stylist<'a>,
}

impl<'a> AstTransformer<'a> {
    /// Create a new transformer for a specific module
    pub fn new(
        transformations: &'a FxHashMap<NodeIndex, Vec<TransformationMetadata>>,
        current_module: ModuleId,
        stylist: &'a ruff_python_codegen::Stylist<'a>,
    ) -> Self {
        Self {
            transformations,
            current_module,
            stylist,
        }
    }

    /// Transform a statement and render it to code
    /// Returns None if the statement should be removed
    pub fn transform_and_render(&self, stmt: &Stmt) -> Option<String> {
        // Get the node index for this statement
        let node_index = stmt.node_index().load();

        // Check if there are transformations for this node
        if let Some(transformations) = self.transformations.get(&node_index) {
            // Apply transformations in priority order
            for transform in transformations {
                match transform {
                    TransformationMetadata::RemoveImport { reason } => {
                        debug!("Removing import due to reason: {reason:?}");
                        return None; // Skip this statement entirely
                    }

                    TransformationMetadata::StdlibImportRewrite {
                        canonical_module,
                        symbols: _,
                    } => {
                        // Generate new import statement
                        let new_import = format!("import {canonical_module}");
                        debug!("Rewriting stdlib import to: {new_import}");
                        return Some(new_import);
                    }

                    TransformationMetadata::PartialImportRemoval {
                        remaining_symbols,
                        removed_symbols,
                    } => {
                        debug!(
                            "Removing symbols {removed_symbols:?}, keeping {remaining_symbols:?}"
                        );

                        if remaining_symbols.is_empty() {
                            return None; // Remove the entire import
                        }

                        // Generate new from-import with remaining symbols
                        return self.render_partial_import(stmt, remaining_symbols);
                    }

                    TransformationMetadata::SymbolRewrite { rewrites } => {
                        // Symbol rewrites are handled during AST traversal
                        trace!("Statement has {} symbol rewrites", rewrites.len());
                    }

                    TransformationMetadata::CircularDepImportMove { .. } => {
                        // Import moves are handled by removing from original location
                        debug!("Removing import for circular dependency move");
                        return None;
                    }
                }
            }
        }

        // If no transformations removed the statement, render it with symbol rewrites
        Some(self.render_with_rewrites(stmt))
    }

    /// Render a partial import with only the remaining symbols
    fn render_partial_import(
        &self,
        stmt: &Stmt,
        remaining_symbols: &[(String, Option<String>)],
    ) -> Option<String> {
        if let Stmt::ImportFrom(import_from) = stmt {
            let module_name = import_from.module.as_ref()?.as_str();

            // Build the import list
            let imports: Vec<String> = remaining_symbols
                .iter()
                .map(|(name, alias)| {
                    if let Some(alias) = alias {
                        format!("{name} as {alias}")
                    } else {
                        name.clone()
                    }
                })
                .collect();

            if imports.is_empty() {
                return None;
            }

            Some(format!(
                "from {} import {}",
                module_name,
                imports.join(", ")
            ))
        } else {
            // Not a from-import, render as-is
            Some(self.render_with_rewrites(stmt))
        }
    }

    /// Render a statement with symbol rewrites applied
    fn render_with_rewrites(&self, stmt: &Stmt) -> String {
        // Clone the statement for mutation
        let mut stmt_clone = stmt.clone();

        // Create a rewrite transformer
        let transformer = RewriteTransformer {
            transformations: self.transformations,
            current_module: self.current_module,
        };

        // Apply the transformer
        transformer.visit_stmt(&mut stmt_clone);

        // Render to Python code
        self.render_statement(&stmt_clone)
    }

    /// Render a statement to Python code
    fn render_statement(&self, stmt: &Stmt) -> String {
        let generator = ruff_python_codegen::Generator::from(self.stylist);
        generator.stmt(stmt)
    }

    /// Create an import statement from ImportData
    pub fn create_import_from_data(&self, data: &ImportData) -> String {
        if data.names.is_empty() {
            // Direct import
            format!("import {}", data.module)
        } else {
            // From import
            let imports: Vec<String> = data
                .names
                .iter()
                .map(|(name, alias)| {
                    if let Some(alias) = alias {
                        format!("{name} as {alias}")
                    } else {
                        name.clone()
                    }
                })
                .collect();

            let dots = ".".repeat(data.level as usize);
            format!("from {}{} import {}", dots, data.module, imports.join(", "))
        }
    }
}

/// Transformer that applies symbol rewrites during AST traversal
struct RewriteTransformer<'a> {
    transformations: &'a FxHashMap<NodeIndex, Vec<TransformationMetadata>>,
    current_module: ModuleId,
}

impl<'a> Transformer for RewriteTransformer<'a> {
    fn visit_expr(&self, expr: &mut Expr) {
        // Get the node index for this expression
        let node_index = expr.node_index().load();

        // Check for symbol rewrites
        if let Some(transformations) = self.transformations.get(&node_index) {
            for transform in transformations {
                if let TransformationMetadata::SymbolRewrite { rewrites } = transform {
                    // Apply rewrite if this node has one
                    if let Some(new_text) = rewrites.get(&node_index) {
                        // Replace the expression based on its type
                        match expr {
                            Expr::Name(_) | Expr::Attribute(_) => {
                                *expr = self.create_name_expr(new_text);
                            }
                            _ => {
                                // Other expression types don't need rewriting
                            }
                        }
                    }
                }
            }
        }

        // Continue traversing
        ruff_python_ast::visitor::transformer::walk_expr(self, expr);
    }
}

impl<'a> RewriteTransformer<'a> {
    /// Create a new Name or Attribute expression with the given text
    fn create_name_expr(&self, name: &str) -> Expr {
        // Check if it's a dotted name (e.g., "typing.Any")
        if let Some(dot_pos) = name.rfind('.') {
            // Split into base and attribute
            let base = &name[..dot_pos];
            let attr = &name[dot_pos + 1..];

            // Recursively create the base expression (handles nested attributes)
            let base_expr = self.create_name_expr(base);

            // Create attribute expression
            Expr::Attribute(ruff_python_ast::ExprAttribute {
                value: Box::new(base_expr),
                attr: ruff_python_ast::Identifier::new(attr, Default::default()),
                ctx: ruff_python_ast::ExprContext::Load,
                range: Default::default(),
                node_index: Default::default(),
            })
        } else {
            // Create a simple name expression
            Expr::Name(ExprName {
                id: name.into(),
                ctx: ruff_python_ast::ExprContext::Load,
                range: Default::default(),
                node_index: Default::default(),
            })
        }
    }
}
