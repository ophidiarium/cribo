use ruff_python_ast::{Stmt, StmtImportFrom};

use crate::code_generator::bundler::Bundler;

pub(in crate::code_generator::import_transformer) fn handle_unbundled_relative_import(
    _bundler: &Bundler,
    import_from: &StmtImportFrom,
    module_name: &str,
    current_module: &str,
) -> Vec<Stmt> {
    // Special case: imports from __main__ modules that aren't the entry
    // These might not be discovered if the __main__.py wasn't explicitly imported
    if module_name.ends_with(".__main__") {
        log::warn!(
            "Relative import 'from {}{}import {:?}' in module '{}' resolves to '{}' which is not \
             bundled. This __main__ module may not have been discovered during bundling.",
            ".".repeat(import_from.level as usize),
            import_from
                .module
                .as_ref()
                .map(|m| format!("{} ", m.as_str()))
                .unwrap_or_default(),
            import_from
                .names
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            current_module,
            module_name
        );
        // Return the original import and let it fail at runtime if the module doesn't exist
        // This is better than panicking during bundling
        return vec![Stmt::ImportFrom(import_from.clone())];
    }

    // Original panic for other non-entry relative imports
    panic!(
        "Relative import 'from {}{}import {:?}' in module '{}' resolves to '{}' which is not \
         bundled or inlined. This is a bug - relative imports are always first-party and should \
         be bundled.",
        ".".repeat(import_from.level as usize),
        import_from
            .module
            .as_ref()
            .map(|m| format!("{} ", m.as_str()))
            .unwrap_or_default(),
        import_from
            .names
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        current_module,
        module_name
    );
}
