// Statement processing utilities and traversal logic for the import transformer
use ruff_python_ast::{AtomicNodeIndex, Expr, Identifier, Stmt, StmtFunctionDef, StmtGlobal};
use ruff_text_size::TextRange;

use crate::{ast_builder::CRIBO_PREFIX, types::FxIndexSet};

/// Statement processing utilities and traversal logic
pub(super) struct StatementProcessor;

impl StatementProcessor {
    /// Collect all names assigned in a target expression.
    /// Supports simple names and destructuring via tuples/lists.
    pub(super) fn collect_assigned_names(target: &Expr, out: &mut FxIndexSet<String>) {
        match target {
            Expr::Name(name) => {
                out.insert(name.id.as_str().to_owned());
            }
            Expr::Tuple(t) => {
                for elt in &t.elts {
                    Self::collect_assigned_names(elt, out);
                }
            }
            Expr::List(l) => {
                for elt in &l.elts {
                    Self::collect_assigned_names(elt, out);
                }
            }
            _ => {}
        }
    }

    /// Check if a condition is a `TYPE_CHECKING` check
    pub(super) fn is_type_checking_condition(expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
            Expr::Attribute(attr) => {
                attr.attr.as_str() == "TYPE_CHECKING"
                    && match &*attr.value {
                        // Check for both typing.TYPE_CHECKING and _cribo.typing.TYPE_CHECKING
                        Expr::Name(name) => name.id.as_str() == "typing",
                        Expr::Attribute(inner_attr) => {
                            // Handle _cribo.typing.TYPE_CHECKING
                            inner_attr.attr.as_str() == "typing"
                                && matches!(&*inner_attr.value, Expr::Name(name) if name.id.as_str() == CRIBO_PREFIX)
                        }
                        _ => false,
                    }
            }
            _ => false,
        }
    }

    /// Move all `global` statements in a function to the start of the function body
    /// (after a leading docstring, if present) and deduplicate their names.
    pub(super) fn hoist_function_globals(func_def: &mut StmtFunctionDef) {
        use ruff_python_ast::helpers::is_docstring_stmt;

        let mut names: FxIndexSet<String> = FxIndexSet::default();
        let mut has_global = false;

        for stmt in &func_def.body {
            if let Stmt::Global(g) = stmt {
                has_global = true;
                for ident in &g.names {
                    names.insert(ident.as_str().to_owned());
                }
            }
        }

        if !has_global {
            return;
        }

        log::debug!(
            "Hoisting {} global name(s) to function start (import transformer)",
            names.len()
        );

        // Remove existing global statements
        let mut new_body: Vec<Stmt> = Vec::with_capacity(func_def.body.len());
        for stmt in func_def.body.drain(..) {
            if !matches!(stmt, Stmt::Global(_)) {
                new_body.push(stmt);
            }
        }

        // Insert after docstring if present
        let insert_at = usize::from(new_body.first().is_some_and(is_docstring_stmt));

        // Build combined global
        let global_stmt = Stmt::Global(StmtGlobal {
            names: names
                .into_iter()
                .map(|s| Identifier::new(s, TextRange::default()))
                .collect(),
            range: TextRange::default(),
            node_index: AtomicNodeIndex::NONE,
        });

        new_body.insert(insert_at, global_stmt);
        func_def.body = new_body;
    }
}
