use ruff_python_ast::{Stmt, StmtImportFrom};

pub fn keep_original_from_import(import_from: &StmtImportFrom) -> Vec<Stmt> {
    vec![Stmt::ImportFrom(import_from.clone())]
}
