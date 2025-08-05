//! Symbol analysis module
//!
//! This module provides analysis capabilities for symbols collected from Python AST,
//! including dependency graph construction, symbol resolution, and export analysis.

use log::debug;
use ruff_python_ast::{Expr, ModModule, Stmt};

use crate::{
    code_generator::{circular_deps::SymbolDependencyGraph, context::HardDependency},
    cribo_graph::CriboGraph as DependencyGraph,
    types::{FxIndexMap, FxIndexSet},
};

/// Symbol analyzer for processing collected symbol data
pub struct SymbolAnalyzer;

impl SymbolAnalyzer {
    /// Collect global symbols from modules (matching bundler's `collect_global_symbols`)
    pub fn collect_global_symbols(
        modules: &[(String, ModModule, std::path::PathBuf, String)],
        entry_module_name: &str,
    ) -> FxIndexSet<String> {
        let mut global_symbols = FxIndexSet::default();

        // Find entry module and collect its top-level symbols
        if let Some((_, ast, _, _)) = modules
            .iter()
            .find(|(name, _, _, _)| name == entry_module_name)
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

    /// Build symbol dependency graph for circular modules
    pub fn build_symbol_dependency_graph(
        modules: &[(String, ModModule, std::path::PathBuf, String)],
        graph: &DependencyGraph,
        circular_modules: &FxIndexSet<String>,
    ) -> SymbolDependencyGraph {
        let mut symbol_dep_graph = SymbolDependencyGraph::default();

        // Collect dependencies for each circular module
        for (module_name, ast, _path, _source) in modules {
            symbol_dep_graph.collect_dependencies(module_name, ast, graph, circular_modules);
        }

        // Only perform topological sort if we have symbols in circular modules
        if symbol_dep_graph.should_sort_symbols(circular_modules)
            && let Err(_) = symbol_dep_graph.topological_sort_symbols(circular_modules)
        {
            // The error is already logged inside topological_sort_symbols
        }

        symbol_dep_graph
    }

    /// Detect hard dependencies in a module
    pub fn detect_hard_dependencies(
        module_name: &str,
        ast: &ModModule,
        import_map: &FxIndexMap<String, (String, Option<String>)>,
    ) -> Vec<HardDependency> {
        let mut hard_deps = Vec::new();

        // Scan for class definitions
        for stmt in &ast.body {
            if let Stmt::ClassDef(class_def) = stmt {
                // Check if any base class is an imported symbol
                if let Some(arguments) = &class_def.arguments {
                    for arg in &arguments.args {
                        hard_deps.extend(Self::check_base_class_dependency(
                            module_name,
                            &class_def.name,
                            arg,
                            import_map,
                            ast,
                        ));
                    }
                }
            }
        }

        hard_deps
    }

    /// Check if a base class expression creates a hard dependency
    fn check_base_class_dependency(
        module_name: &str,
        class_name: &str,
        base_expr: &Expr,
        import_map: &FxIndexMap<String, (String, Option<String>)>,
        ast: &ModModule,
    ) -> Vec<HardDependency> {
        let mut deps = Vec::new();

        match base_expr {
            // Handle requests.compat.MutableMapping style
            Expr::Attribute(attr_expr) => {
                if let Expr::Attribute(inner_attr) = &*attr_expr.value {
                    if let Expr::Name(name_expr) = &*inner_attr.value {
                        let base_module = name_expr.id.as_str();
                        let sub_module = inner_attr.attr.as_str();
                        let attr_name = attr_expr.attr.as_str();

                        // Check if this module.submodule is in our import map
                        let full_module = format!("{base_module}.{sub_module}");
                        if let Some((source_module, _alias)) = import_map.get(&full_module) {
                            debug!(
                                "Found hard dependency: class {class_name} in module \
                                 {module_name} inherits from \
                                 {base_module}.{sub_module}.{attr_name}"
                            );

                            deps.push(HardDependency {
                                module_name: module_name.to_string(),
                                class_name: class_name.to_string(),
                                base_class: format!("{base_module}.{sub_module}.{attr_name}"),
                                source_module: source_module.clone(),
                                imported_attr: attr_name.to_string(),
                                alias: None,
                                alias_is_mandatory: false,
                            });
                        }
                    }
                } else if let Expr::Name(name_expr) = &*attr_expr.value {
                    // Handle simple module.Class style
                    let module = name_expr.id.as_str();
                    let class = attr_expr.attr.as_str();

                    if let Some((source_module, _alias)) = import_map.get(module) {
                        debug!(
                            "Found hard dependency: class {class_name} in module {module_name} \
                             inherits from {module}.{class}"
                        );

                        // For module.attr, we import the module itself, not the attribute
                        // No alias is needed for module.attr imports
                        deps.push(HardDependency {
                            module_name: module_name.to_string(),
                            class_name: class_name.to_string(),
                            base_class: format!("{module}.{class}"),
                            source_module: source_module.clone(),
                            imported_attr: module.to_string(), // Import the module, not the attr
                            alias: None,                       // No alias for module.attr imports
                            alias_is_mandatory: false,
                        });
                    }
                }
            }
            // Handle direct name references
            Expr::Name(name_expr) => {
                let base_name = name_expr.id.as_str();

                // Check if this is an imported class
                if let Some((source_module, original_name)) = import_map.get(base_name) {
                    debug!(
                        "Found hard dependency: class {class_name} in module {module_name} \
                         inherits from {base_name}"
                    );

                    // Use the original imported name if available (for aliased imports)
                    let import_attr = original_name
                        .clone()
                        .unwrap_or_else(|| base_name.to_string());

                    // Check if this base_name is used as an alias
                    // If base_name != import_attr, then base_name is an alias
                    let has_alias = base_name != import_attr;

                    // Check if the alias is mandatory (i.e., the original name
                    // conflicts with a local definition)
                    let alias_is_mandatory = if has_alias {
                        // Check if there's a local class with the same name as import_attr
                        crate::code_generator::module_registry::check_local_name_conflict(
                            ast,
                            &import_attr,
                        )
                    } else {
                        false
                    };

                    deps.push(HardDependency {
                        module_name: module_name.to_string(),
                        class_name: class_name.to_string(),
                        base_class: base_name.to_string(),
                        source_module: source_module.clone(),
                        imported_attr: import_attr,
                        alias: if has_alias {
                            Some(base_name.to_string())
                        } else {
                            None
                        },
                        alias_is_mandatory,
                    });
                }
            }
            _ => {}
        }

        deps
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
        module_name: &str,
        kept_symbols: Option<&FxIndexMap<String, FxIndexSet<String>>>,
        enable_logging: bool,
    ) -> Vec<&'a String> {
        if let Some(kept_symbols) = kept_symbols {
            let result: Vec<&String> = exports
                .iter()
                .filter(|symbol| {
                    // Check if this symbol is kept in this module
                    // With the new data structure, we can do efficient lookups without allocations
                    let is_kept = kept_symbols
                        .get(module_name)
                        .is_some_and(|symbols| symbols.contains(*symbol));

                    if enable_logging {
                        let (action, preposition, reason) = if is_kept {
                            ("Keeping", "in", "survived")
                        } else {
                            ("Filtering out", "from", "removed by")
                        };
                        debug!(
                            "{action} symbol '{symbol}' {preposition} __all__ of module \
                             '{module_name}' - {reason} tree-shaking"
                        );
                    }

                    is_kept
                })
                .collect();

            if enable_logging {
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

        let modules = vec![(
            "test_module".to_string(),
            module,
            std::path::PathBuf::new(),
            "hash".to_string(),
        )];

        let symbols = SymbolAnalyzer::collect_global_symbols(&modules, "test_module");

        assert_eq!(symbols.len(), 3);
        assert!(symbols.contains("main"));
        assert!(symbols.contains("Config"));
        assert!(symbols.contains("VERSION"));
    }

    #[test]
    fn test_detect_hard_dependencies() {
        let code = r"
import base_module
from typing import Protocol

class MyClass(base_module.BaseClass):
    pass

class MyProtocol(Protocol):
    pass
";
        let parsed = parse_module(code).expect("Failed to parse test module");
        let module = parsed.into_syntax();

        let mut import_map = FxIndexMap::default();
        import_map.insert("base_module".to_string(), ("base_module".to_string(), None));
        import_map.insert("Protocol".to_string(), ("typing".to_string(), None));

        let hard_deps =
            SymbolAnalyzer::detect_hard_dependencies("test_module", &module, &import_map);

        assert_eq!(hard_deps.len(), 2);

        let first_dep = &hard_deps[0];
        assert_eq!(first_dep.class_name, "MyClass");
        assert_eq!(first_dep.base_class, "base_module.BaseClass");
        assert_eq!(first_dep.source_module, "base_module");

        let second_dep = &hard_deps[1];
        assert_eq!(second_dep.class_name, "MyProtocol");
        assert_eq!(second_dep.base_class, "Protocol");
        assert_eq!(second_dep.source_module, "typing");
    }
}
