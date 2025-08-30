//! Symbol analysis module
//!
//! This module provides analysis capabilities for symbols collected from Python AST,
//! including dependency graph construction, symbol resolution, and export analysis.

use log::debug;
use ruff_python_ast::{Expr, ModModule, Stmt};

use crate::types::{FxIndexMap, FxIndexSet};

/// Symbol analyzer for processing collected symbol data
pub struct SymbolAnalyzer;

impl SymbolAnalyzer {
    /// Collect global symbols from modules (matching bundler's `collect_global_symbols`)
    pub fn collect_global_symbols(
        modules: &[(
            crate::resolver::ModuleId,
            ModModule,
            std::path::PathBuf,
            String,
        )],
    ) -> FxIndexSet<String> {
        let mut global_symbols = FxIndexSet::default();

        // Find entry module and collect its top-level symbols
        if let Some((_, ast, _, _)) = modules
            .iter()
            .find(|(module_id, _, _, _)| module_id.is_entry())
        {
            for stmt in &ast.body {
                match stmt {
                    Stmt::FunctionDef(func_def) => {
                        global_symbols.insert(func_def.name.to_string());
                    }
                    Stmt::ClassDef(class_def) => {
                        global_symbols.insert(class_def.name.to_string());
                    }
                    Stmt::Assign(assign) => {
                        for target in &assign.targets {
                            if let Expr::Name(name) = target {
                                global_symbols.insert(name.id.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        global_symbols
    }

    /// Filter exports based on tree shaking
    ///
    /// This function filters a list of export symbols based on whether they survived tree-shaking.
    /// It optionally logs debug information about which symbols were kept or filtered.
    ///
    /// # Arguments
    /// * `exports` - The list of export symbols to filter
    /// * `module_name` - The name of the module these exports belong to
    /// * `kept_symbols` - Optional map from module name to a set of symbols to keep in that module
    /// * `enable_logging` - Whether to log debug information about kept/filtered symbols
    ///
    /// # Returns
    /// A vector of references to the export symbols that should be kept
    pub fn filter_exports_by_tree_shaking<'a>(
        exports: &'a [String],
        module_id: &crate::resolver::ModuleId,
        kept_symbols: Option<&FxIndexMap<crate::resolver::ModuleId, FxIndexSet<String>>>,
        enable_logging: bool,
        resolver: &crate::resolver::ModuleResolver,
    ) -> Vec<&'a String> {
        if let Some(kept_symbols) = kept_symbols {
            let result: Vec<&String> = exports
                .iter()
                .filter(|symbol| {
                    // Check if this symbol is kept in this module
                    // With the new data structure, we can do efficient lookups without allocations
                    let is_kept = kept_symbols
                        .get(module_id)
                        .is_some_and(|symbols| symbols.contains(*symbol));

                    if enable_logging {
                        let (action, preposition, reason) = if is_kept {
                            ("Keeping", "in", "survived")
                        } else {
                            ("Filtering out", "from", "removed by")
                        };
                        let module_name = resolver
                            .get_module_name(*module_id)
                            .expect("Module name must exist for ModuleId");
                        debug!(
                            "{action} symbol '{symbol}' {preposition} __all__ of module \
                             '{module_name}' - {reason} tree-shaking"
                        );
                    }

                    is_kept
                })
                .collect();

            if enable_logging {
                let module_name = resolver
                    .get_module_name(*module_id)
                    .expect("Module name must exist for ModuleId");
                debug!(
                    "Module '{}' __all__ filtering: {} symbols -> {} symbols",
                    module_name,
                    exports.len(),
                    result.len()
                );
            }

            result
        } else {
            // No tree-shaking, include all exports
            exports.iter().collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_collect_global_symbols() {
        let code = r#"
def main():
    pass

class Config:
    pass

VERSION = "1.0.0"
"#;
        let parsed = parse_module(code).expect("Failed to parse test module");
        let module = parsed.into_syntax();

        // Create a ModuleId for the test
        let module_id = crate::resolver::ModuleId::new(0);
        let modules = vec![(
            module_id,
            module,
            std::path::PathBuf::new(),
            "hash".to_string(),
        )];

        let symbols = SymbolAnalyzer::collect_global_symbols(&modules);

        assert_eq!(symbols.len(), 3);
        assert!(symbols.contains("main"));
        assert!(symbols.contains("Config"));
        assert!(symbols.contains("VERSION"));
    }
}
