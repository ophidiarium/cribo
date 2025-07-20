//! Symbol analysis module
//!
//! This module provides analysis capabilities for symbols collected from Python AST,
//! including dependency graph construction, symbol resolution, and export analysis.

use log::debug;
use ruff_python_ast::{Expr, ModModule, Stmt};

use crate::{
    analyzers::types::{CollectedSymbols, SymbolAnalysis},
    code_generator::{circular_deps::SymbolDependencyGraph, context::HardDependency},
    cribo_graph::CriboGraph as DependencyGraph,
    types::{FxIndexMap, FxIndexSet},
    visitors::{SymbolCollector, VariableCollector},
};

/// Symbol analyzer for processing collected symbol data
pub struct SymbolAnalyzer;

impl SymbolAnalyzer {
    /// Analyze a module and return comprehensive symbol analysis
    pub fn analyze_module(module: &ModModule) -> SymbolAnalysis {
        // Run visitors to collect data
        let symbols = SymbolCollector::analyze(module);
        let variables = VariableCollector::analyze(module);

        // Build symbol dependencies (simplified for now)
        let symbol_dependencies = Self::build_symbol_dependencies(&symbols);

        // Extract export information
        let exports = Self::extract_exports(&symbols);

        SymbolAnalysis {
            symbols,
            variables,
            exports,
            symbol_dependencies,
        }
    }

    /// Collect global symbols from modules (matching bundler's collect_global_symbols)
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
            let collected = SymbolCollector::analyze(ast);

            // Add all global symbols from the entry module
            for (name, _) in collected.global_symbols {
                global_symbols.insert(name);
            }
        }

        global_symbols
    }

    /// Find which module defines a given symbol
    pub fn find_symbol_module(
        symbol: &str,
        current_module: &str,
        graph: &DependencyGraph,
        circular_modules: &FxIndexSet<String>,
    ) -> Option<String> {
        // First check if it's defined in the current module
        if let Some(module_dep_graph) = graph.get_module_by_name(current_module) {
            for item_data in module_dep_graph.items.values() {
                match &item_data.item_type {
                    crate::cribo_graph::ItemType::FunctionDef { name } if name == symbol => {
                        return Some(current_module.to_string());
                    }
                    crate::cribo_graph::ItemType::ClassDef { name } if name == symbol => {
                        return Some(current_module.to_string());
                    }
                    crate::cribo_graph::ItemType::Assignment { targets } => {
                        if targets.contains(&symbol.to_string()) {
                            return Some(current_module.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }

        // Check other circular modules
        for module_name in circular_modules {
            if module_name == current_module {
                continue;
            }

            if let Some(module_dep_graph) = graph.get_module_by_name(module_name) {
                for item_data in module_dep_graph.items.values() {
                    match &item_data.item_type {
                        crate::cribo_graph::ItemType::FunctionDef { name } if name == symbol => {
                            return Some(module_name.clone());
                        }
                        crate::cribo_graph::ItemType::ClassDef { name } if name == symbol => {
                            return Some(module_name.clone());
                        }
                        crate::cribo_graph::ItemType::Assignment { targets } => {
                            if targets.contains(&symbol.to_string()) {
                                return Some(module_name.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        None
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
            && let Err(e) = symbol_dep_graph.topological_sort_symbols(circular_modules)
        {
            // The error is already logged inside topological_sort_symbols
            log::error!("Failed to sort symbols: {e}");
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

                    if let Some((source_module, alias)) = import_map.get(module) {
                        debug!(
                            "Found hard dependency: class {class_name} in module {module_name} \
                             inherits from {module}.{class}"
                        );

                        let alias_is_mandatory = alias.is_some();
                        deps.push(HardDependency {
                            module_name: module_name.to_string(),
                            class_name: class_name.to_string(),
                            base_class: format!("{module}.{class}"),
                            source_module: source_module.clone(),
                            imported_attr: class.to_string(),
                            alias: alias.clone(),
                            alias_is_mandatory,
                        });
                    }
                }
            }
            // Handle direct name references
            Expr::Name(name_expr) => {
                let base_name = name_expr.id.as_str();

                // Check if this is an imported class
                if let Some((source_module, alias)) = import_map.get(base_name) {
                    debug!(
                        "Found hard dependency: class {class_name} in module {module_name} \
                         inherits from {base_name}"
                    );

                    deps.push(HardDependency {
                        module_name: module_name.to_string(),
                        class_name: class_name.to_string(),
                        base_class: base_name.to_string(),
                        source_module: source_module.clone(),
                        imported_attr: base_name.to_string(),
                        alias: alias.clone(),
                        alias_is_mandatory: false,
                    });
                }
            }
            _ => {}
        }

        deps
    }

    /// Build symbol dependencies from collected symbols
    fn build_symbol_dependencies(
        symbols: &CollectedSymbols,
    ) -> FxIndexMap<String, FxIndexSet<String>> {
        let mut dependencies = FxIndexMap::default();

        // For now, return empty dependencies
        // This will be enhanced when we have the variable collector
        for (name, _) in &symbols.global_symbols {
            dependencies.insert(name.clone(), FxIndexSet::default());
        }

        dependencies
    }

    /// Extract export information from collected symbols
    fn extract_exports(symbols: &CollectedSymbols) -> Option<crate::analyzers::types::ExportInfo> {
        // Check if any symbols have explicit export information
        let exported_symbols: Vec<String> = symbols
            .global_symbols
            .values()
            .filter(|s| s.is_exported)
            .map(|s| s.name.clone())
            .collect();

        if exported_symbols.is_empty() {
            None
        } else {
            Some(crate::analyzers::types::ExportInfo {
                exported_names: Some(exported_symbols),
                is_dynamic: false,
                re_exports: Vec::new(),
            })
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
        let code = r#"
import base_module
from typing import Protocol

class MyClass(base_module.BaseClass):
    pass

class MyProtocol(Protocol):
    pass
"#;
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
