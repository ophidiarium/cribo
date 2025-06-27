//! Circular dependency analyzer that works with structured types
//!
//! This module provides the analysis logic for circular dependencies,
//! working directly with CriboGraph and producing structured results.

use rustc_hash::FxHashMap;

use super::circular_deps::{
    CircularDependencyAnalysis, CircularDependencyGroup, CircularDependencyType, CycleMetadata,
    EdgeMetadata, EdgeType, ModuleEdge, ResolutionStrategy,
};
use crate::cribo_graph::{CriboGraph, ItemType, ModuleId};

/// Analyzer for circular dependencies
pub struct CircularDependencyAnalyzer<'a> {
    graph: &'a CriboGraph,
}

impl<'a> CircularDependencyAnalyzer<'a> {
    /// Create a new analyzer for the given graph
    pub fn new(graph: &'a CriboGraph) -> Self {
        Self { graph }
    }

    /// Analyze circular dependencies and return structured results
    pub fn analyze(&self) -> CircularDependencyAnalysis {
        let sccs = self.graph.find_strongly_connected_components();
        let cycle_paths = self.find_cycle_paths_as_module_ids();

        let mut resolvable_cycles = Vec::new();
        let mut unresolvable_cycles = Vec::new();
        let mut largest_cycle_size = 0;
        let total_cycles_detected = sccs.len();

        for scc in &sccs {
            if scc.len() > largest_cycle_size {
                largest_cycle_size = scc.len();
            }

            // Build import chain for the SCC
            let import_chain = self.build_import_chain_for_scc(scc);

            // Analyze the cycle
            let metadata = self.analyze_cycle_metadata(scc, &import_chain);

            // Classify the cycle type
            let cycle_type = self.classify_cycle_type(scc, &import_chain, &metadata);

            // Suggest resolution strategy
            let suggested_resolution =
                self.suggest_resolution_for_cycle(&cycle_type, scc, &import_chain, &metadata);

            let group = CircularDependencyGroup {
                module_ids: scc.to_vec(),
                cycle_type: cycle_type.clone(),
                import_chain,
                suggested_resolution,
                metadata,
            };

            // Categorize based on cycle type
            match cycle_type {
                CircularDependencyType::ModuleConstants => {
                    unresolvable_cycles.push(group);
                }
                _ => {
                    resolvable_cycles.push(group);
                }
            }
        }

        CircularDependencyAnalysis {
            resolvable_cycles,
            unresolvable_cycles,
            total_cycles_detected,
            largest_cycle_size,
            cycle_paths,
        }
    }

    /// Find cycle paths and return them as ModuleIds
    fn find_cycle_paths_as_module_ids(&self) -> Vec<Vec<ModuleId>> {
        // Use the existing method but don't convert to strings
        self.graph.find_cycles()
    }

    /// Build import chain for a strongly connected component
    fn build_import_chain_for_scc(&self, scc: &[ModuleId]) -> Vec<ModuleEdge> {
        let mut import_chain = Vec::new();

        for &from_module_id in scc {
            // Get dependencies of this module that are also in the SCC
            let deps = self.graph.get_dependencies(from_module_id);
            for to_module_id in deps {
                if !scc.contains(&to_module_id) {
                    continue;
                }

                // Analyze imports to find all edge types and metadata
                let edges = self.analyze_import_edges(from_module_id, to_module_id);
                import_chain.extend(edges);
            }
        }

        import_chain
    }

    /// Analyze all import edges between two modules
    /// Returns all import statements from from_id to to_id
    fn analyze_import_edges(&self, from_id: ModuleId, to_id: ModuleId) -> Vec<ModuleEdge> {
        let mut edges = Vec::new();

        let Some(from_module) = self.graph.modules.get(&from_id) else {
            return edges;
        };

        // Find all import statement(s) that create edges to the target module
        for (item_id, item_data) in &from_module.items {
            match &item_data.item_type {
                ItemType::Import { module, alias } => {
                    if self.graph.module_names.get(module) == Some(&to_id) {
                        edges.push(ModuleEdge {
                            from_module: from_id,
                            to_module: to_id,
                            edge_type: if let Some(alias) = alias {
                                EdgeType::AliasedImport {
                                    alias: alias.clone(),
                                }
                            } else {
                                EdgeType::DirectImport
                            },
                            metadata: EdgeMetadata {
                                line_number: item_data.span.map(|(start, _)| start),
                                import_item_id: Some(*item_id),
                                is_module_level: true, // TODO: check scoping
                                containing_function: None, // TODO: check scoping
                            },
                        });
                    }
                }
                ItemType::FromImport {
                    module,
                    names,
                    level,
                    ..
                } => {
                    if self.graph.module_names.get(module) == Some(&to_id) {
                        let symbols: Vec<String> =
                            names.iter().map(|(name, _alias)| name.clone()).collect();

                        edges.push(ModuleEdge {
                            from_module: from_id,
                            to_module: to_id,
                            edge_type: if *level > 0 {
                                EdgeType::RelativeImport {
                                    level: *level,
                                    symbols,
                                }
                            } else {
                                EdgeType::FromImport { symbols }
                            },
                            metadata: EdgeMetadata {
                                line_number: item_data.span.map(|(start, _)| start),
                                import_item_id: Some(*item_id),
                                is_module_level: true, // TODO: check scoping
                                containing_function: None, // TODO: check scoping
                            },
                        });
                    }
                }
                _ => {}
            }
        }

        edges
    }

    /// Analyze metadata about a cycle
    fn analyze_cycle_metadata(
        &self,
        module_ids: &[ModuleId],
        import_chain: &[ModuleEdge],
    ) -> CycleMetadata {
        let mut all_function_scoped = true;
        let mut involves_classes = false;
        let mut has_module_constants = false;
        let mut complexity_score = 0;

        for &module_id in module_ids {
            let Some(module) = self.graph.modules.get(&module_id) else {
                continue;
            };

            // Check if module has class definitions
            if module
                .items
                .values()
                .any(|item| matches!(item.item_type, ItemType::ClassDef { .. }))
            {
                involves_classes = true;
                complexity_score += 10;
            }

            // Check if module only has constants
            if self.module_has_only_constants(module_id) {
                has_module_constants = true;
                complexity_score += 20;
            }

            // Check import scoping
            for edge in import_chain {
                if edge.from_module == module_id && edge.metadata.is_module_level {
                    all_function_scoped = false;
                }
            }
        }

        // Add complexity based on cycle size
        complexity_score += (module_ids.len() as u32) * 5;

        CycleMetadata {
            all_function_scoped,
            involves_classes,
            has_module_constants,
            complexity_score,
        }
    }

    /// Check if a module only contains constant assignments
    fn module_has_only_constants(&self, module_id: ModuleId) -> bool {
        let Some(module) = self.graph.modules.get(&module_id) else {
            return false;
        };

        // Empty modules or modules with only imports are not "only constants"
        !module.items.is_empty()
            && module
                .items
                .values()
                .any(|item| matches!(item.item_type, ItemType::Assignment { .. }))
            && !module.items.values().any(|item| {
                matches!(
                    &item.item_type,
                    ItemType::FunctionDef { .. }
                        | ItemType::ClassDef { .. }
                        | ItemType::Expression
                        | ItemType::If { .. }
                        | ItemType::Try
                )
            })
    }

    /// Classify the type of circular dependency
    fn classify_cycle_type(
        &self,
        module_ids: &[ModuleId],
        _import_chain: &[ModuleEdge],
        metadata: &CycleMetadata,
    ) -> CircularDependencyType {
        // Check if this is a parent-child package cycle
        if self.is_parent_child_package_cycle(module_ids) {
            return CircularDependencyType::FunctionLevel;
        }

        // Use metadata for classification
        if metadata.has_module_constants && !self.any_module_is_init(module_ids) {
            return CircularDependencyType::ModuleConstants;
        }

        if metadata.involves_classes {
            if metadata.all_function_scoped {
                return CircularDependencyType::FunctionLevel;
            }
            return CircularDependencyType::ClassLevel;
        }

        // Check if all modules are empty or only have imports
        if self.all_modules_empty_or_imports_only(module_ids) {
            return CircularDependencyType::FunctionLevel;
        }

        if metadata.all_function_scoped {
            CircularDependencyType::FunctionLevel
        } else if self.any_module_is_init(module_ids) {
            CircularDependencyType::ImportTime
        } else {
            CircularDependencyType::FunctionLevel
        }
    }

    /// Check if any module in the list is an __init__ module
    fn any_module_is_init(&self, module_ids: &[ModuleId]) -> bool {
        module_ids.iter().any(|&id| {
            self.graph
                .modules
                .get(&id)
                .map(|m| m.module_name.ends_with("__init__"))
                .unwrap_or(false)
        })
    }

    /// Check if this is a parent-child package cycle
    fn is_parent_child_package_cycle(&self, module_ids: &[ModuleId]) -> bool {
        let module_names: Vec<_> = module_ids
            .iter()
            .filter_map(|&id| self.graph.modules.get(&id).map(|m| &m.module_name))
            .collect();

        for (i, name1) in module_names.iter().enumerate() {
            for name2 in module_names.iter().skip(i + 1) {
                if name1.starts_with(&format!("{name2}."))
                    || name2.starts_with(&format!("{name1}."))
                {
                    return true;
                }
            }
        }
        false
    }

    /// Check if all modules are empty or only contain imports
    fn all_modules_empty_or_imports_only(&self, module_ids: &[ModuleId]) -> bool {
        module_ids.iter().all(|&id| {
            self.graph
                .modules
                .get(&id)
                .map(|module| {
                    module.items.is_empty()
                        || module.items.values().all(|item| {
                            matches!(
                                item.item_type,
                                ItemType::Import { .. } | ItemType::FromImport { .. }
                            )
                        })
                })
                .unwrap_or(true)
        })
    }

    /// Suggest resolution strategy for a cycle
    fn suggest_resolution_for_cycle(
        &self,
        cycle_type: &CircularDependencyType,
        module_ids: &[ModuleId],
        _import_chain: &[ModuleEdge],
        _metadata: &CycleMetadata,
    ) -> ResolutionStrategy {
        match cycle_type {
            CircularDependencyType::FunctionLevel => {
                // Find imports that can be moved to function scope
                let import_to_function = FxHashMap::default();
                let mut descriptions = Vec::new();

                for edge in _import_chain {
                    if let Some(_import_item_id) = edge.metadata.import_item_id {
                        // TODO: Find appropriate function to move import into
                        descriptions.push(format!(
                            "Move import of module {:?} to function scope",
                            edge.to_module
                        ));
                    }
                }

                ResolutionStrategy::FunctionScopedImport {
                    import_to_function,
                    descriptions,
                }
            }
            CircularDependencyType::ClassLevel => {
                // Suggest lazy imports for class-level cycles
                let mut lazy_var_names = FxHashMap::default();
                for &module_id in module_ids {
                    if let Some(module) = self.graph.modules.get(&module_id) {
                        let lazy_name = format!("_lazy_{}", module.module_name.replace('.', "_"));
                        lazy_var_names.insert(module_id, lazy_name);
                    }
                }

                ResolutionStrategy::LazyImport {
                    module_ids: module_ids.to_vec(),
                    lazy_var_names,
                }
            }
            CircularDependencyType::ModuleConstants => ResolutionStrategy::Unresolvable {
                reason: "Circular dependency between module-level constants".to_string(),
                manual_suggestions: vec![
                    "Consider moving constants to a separate module".to_string(),
                    "Refactor to avoid circular constant dependencies".to_string(),
                ],
            },
            CircularDependencyType::ImportTime => ResolutionStrategy::Unresolvable {
                reason: "Import-time circular dependency detected".to_string(),
                manual_suggestions: vec![
                    "Consider restructuring module hierarchy".to_string(),
                    "Move imports to function scope where possible".to_string(),
                ],
            },
        }
    }
}
