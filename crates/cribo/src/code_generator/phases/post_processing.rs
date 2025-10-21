//! Post-Processing Phase
//!
//! This phase handles final transformations after all modules are processed:
//! - Namespace attachment for entry module exports
//! - Proxy generation for stdlib access
//! - Package child alias generation

use ruff_python_ast::Stmt;

use crate::{
    code_generator::{bundler::Bundler, context::PostProcessingResult},
    types::{FxIndexMap, FxIndexSet},
};

/// Post-processing phase handler (stateless)
#[derive(Default)]
pub(crate) struct PostProcessingPhase;

impl PostProcessingPhase {
    /// Create a new post-processing phase
    pub(crate) const fn new() -> Self {
        Self
    }

    /// Execute the post-processing phase
    ///
    /// This method:
    /// 1. Attaches entry module exports to namespace (for packages)
    /// 2. Generates proxy statements for stdlib access
    /// 3. Generates package child alias statements
    ///
    /// Returns statements to be inserted at appropriate positions in the final bundle.
    pub(crate) fn execute(
        &self,
        bundler: &mut Bundler<'_>,
        entry_symbols: &FxIndexSet<String>,
        entry_renames: &FxIndexMap<String, String>,
        final_body: &[Stmt],
    ) -> PostProcessingResult {
        // Generate namespace attachments for entry module exports
        let namespace_attachments =
            Self::generate_namespace_attachments(bundler, entry_symbols, entry_renames);

        // Generate proxy statements for stdlib access
        let proxy_statements = Self::generate_proxy_statements();

        // Generate package child aliases
        let alias_statements = Self::generate_package_child_aliases(bundler, final_body);

        PostProcessingResult {
            proxy_statements,
            alias_statements,
            namespace_attachments,
        }
    }

    /// Generate namespace attachment statements for entry module exports
    fn generate_namespace_attachments(
        bundler: &mut Bundler<'_>,
        entry_symbols: &FxIndexSet<String>,
        entry_renames: &FxIndexMap<String, String>,
    ) -> Vec<Stmt> {
        log::debug!(
            "Checking if entry module needs namespace attachment: \
             entry_is_package_init_or_main={}, entry_module_name='{}'",
            bundler.entry_is_package_init_or_main,
            bundler.entry_module_name
        );

        if !bundler.entry_is_package_init_or_main {
            return Vec::new();
        }

        let entry_pkg = bundler
            .entry_package_name()
            .map(ToString::to_string)
            .or_else(|| bundler.infer_entry_root_package())
            .unwrap_or_else(|| bundler.entry_module_name.clone());

        if entry_pkg.is_empty() || entry_pkg == crate::python::constants::MAIN_STEM {
            log::warn!(
                "Skipping namespace attachment: ambiguous entry package for '{}'",
                bundler.entry_module_name
            );
            return Vec::new();
        }

        log::debug!("Using package name '{entry_pkg}' for namespace attachment");

        let mut attachments = Vec::new();
        bundler.emit_entry_namespace_attachments(
            &entry_pkg,
            &mut attachments,
            entry_symbols,
            entry_renames,
        );
        attachments
    }

    /// Generate proxy statements for stdlib access
    fn generate_proxy_statements() -> Vec<Stmt> {
        log::debug!("Generating _cribo proxy for stdlib access");
        crate::ast_builder::proxy_generator::generate_cribo_proxy()
    }

    /// Generate package child alias statements
    fn generate_package_child_aliases(bundler: &Bundler<'_>, final_body: &[Stmt]) -> Vec<Stmt> {
        use ruff_python_ast::ExprContext;

        use crate::{
            ast_builder::{expressions, statements},
            python::constants::INIT_STEM,
        };

        let mut alias_statements = Vec::new();

        let entry_pkg = bundler
            .infer_entry_root_package()
            .unwrap_or_else(|| bundler.entry_module_name.clone());

        if entry_pkg.is_empty() || entry_pkg == INIT_STEM {
            return alias_statements;
        }

        // Collect simple names already defined
        let existing_variables: FxIndexSet<String> = final_body
            .iter()
            .filter_map(|stmt| {
                if let Stmt::Assign(assign) = stmt
                    && let [ruff_python_ast::Expr::Name(name)] = assign.targets.as_slice()
                {
                    Some(name.id.to_string())
                } else {
                    None
                }
            })
            .collect();

        // Add aliases for all direct child modules
        let mut seen: FxIndexSet<String> = FxIndexSet::default();
        let mut added = 0_usize;

        for child in bundler
            .bundled_modules
            .iter()
            .filter_map(|id| bundler.resolver.get_module_name(*id))
        {
            if let Some(rest) = child.strip_prefix(&format!("{entry_pkg}.")) {
                let first = rest.split('.').next().unwrap_or("");
                if first.is_empty() || first.starts_with('_') {
                    continue;
                }
                if !seen.insert(first.to_owned()) {
                    continue;
                }
                if existing_variables.contains(first) {
                    log::debug!(
                        "Post-pass: skipping alias for {child} as '{first}' (would overwrite)"
                    );
                    continue;
                }

                log::debug!("Post-pass: adding alias '{first} = {entry_pkg}.{first}'");
                alias_statements.push(statements::simple_assign(
                    first,
                    expressions::attribute(
                        expressions::name(&entry_pkg, ExprContext::Load),
                        first,
                        ExprContext::Load,
                    ),
                ));
                added += 1;
            }
        }

        log::debug!("Post-pass: added {added} module-level aliases for package '{entry_pkg}'");
        alias_statements
    }

    /// Insert proxy statements after __future__ imports
    pub(crate) fn insert_proxy_statements(proxy_statements: Vec<Stmt>, final_body: &mut Vec<Stmt>) {
        log::debug!("Inserting _cribo proxy after __future__ imports");

        // Find position after optional module docstring and __future__ imports
        // Skip leading module docstring
        let mut insert_position = if let Some(Stmt::Expr(expr)) = final_body.first()
            && matches!(expr.value.as_ref(), ruff_python_ast::Expr::StringLiteral(_))
        {
            1
        } else {
            0
        };

        // Skip contiguous __future__ imports after docstring
        for (i, stmt) in final_body.iter().enumerate().skip(insert_position) {
            if let Stmt::ImportFrom(import_from) = stmt
                && let Some(module) = &import_from.module
                && module.as_str() == "__future__"
            {
                insert_position = i + 1;
                continue;
            }
            break;
        }

        // Insert proxy statements
        for (i, stmt) in proxy_statements.into_iter().enumerate() {
            final_body.insert(insert_position + i, stmt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_post_processing_output_construction() {
        let output = PostProcessingResult {
            proxy_statements: vec![],
            alias_statements: vec![],
            namespace_attachments: vec![],
        };

        assert!(output.proxy_statements.is_empty());
        assert!(output.alias_statements.is_empty());
        assert!(output.namespace_attachments.is_empty());
    }

    #[test]
    fn test_post_processing_with_statements() {
        use ruff_python_ast::{AtomicNodeIndex, StmtExpr};
        use ruff_text_size::TextRange;

        let stmt = Stmt::Expr(StmtExpr {
            node_index: AtomicNodeIndex::NONE,
            range: TextRange::default(),
            value: Box::new(ruff_python_ast::Expr::NumberLiteral(
                ruff_python_ast::ExprNumberLiteral {
                    node_index: AtomicNodeIndex::NONE,
                    range: TextRange::default(),
                    value: ruff_python_ast::Number::Int(ruff_python_ast::Int::ZERO),
                },
            )),
        });

        let output = PostProcessingResult {
            proxy_statements: vec![stmt.clone()],
            alias_statements: vec![stmt.clone()],
            namespace_attachments: vec![stmt],
        };

        assert_eq!(output.proxy_statements.len(), 1);
        assert_eq!(output.alias_statements.len(), 1);
        assert_eq!(output.namespace_attachments.len(), 1);
    }

    #[test]
    fn test_insert_proxy_statements_empty() {
        let proxy_statements = vec![];
        let mut final_body = vec![];

        PostProcessingPhase::insert_proxy_statements(proxy_statements, &mut final_body);

        assert!(final_body.is_empty());
    }

    #[test]
    fn test_insert_proxy_statements_at_beginning() {
        use ruff_python_ast::{AtomicNodeIndex, StmtExpr};
        use ruff_text_size::TextRange;

        let proxy_stmt = Stmt::Expr(StmtExpr {
            node_index: AtomicNodeIndex::NONE,
            range: TextRange::default(),
            value: Box::new(ruff_python_ast::Expr::NumberLiteral(
                ruff_python_ast::ExprNumberLiteral {
                    node_index: AtomicNodeIndex::NONE,
                    range: TextRange::default(),
                    value: ruff_python_ast::Number::Int(ruff_python_ast::Int::ONE),
                },
            )),
        });

        let original_stmt = Stmt::Expr(StmtExpr {
            node_index: AtomicNodeIndex::NONE,
            range: TextRange::default(),
            value: Box::new(ruff_python_ast::Expr::NumberLiteral(
                ruff_python_ast::ExprNumberLiteral {
                    node_index: AtomicNodeIndex::NONE,
                    range: TextRange::default(),
                    value: ruff_python_ast::Number::Int(ruff_python_ast::Int::ZERO),
                },
            )),
        });

        let mut final_body = vec![original_stmt];
        let proxy_statements = vec![proxy_stmt];

        PostProcessingPhase::insert_proxy_statements(proxy_statements, &mut final_body);

        // Proxy should be inserted at position 0 (no __future__ imports)
        assert_eq!(final_body.len(), 2);
    }
}
