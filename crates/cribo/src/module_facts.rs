use anyhow::Result;
use ruff_python_ast::{ModModule, visitor::source_order::SourceOrderVisitor};

use crate::{
    dependency_graph::{ItemData, ModuleDepGraph},
    graph_builder::GraphBuilder,
    resolver::ModuleId,
    visitors::{DiscoveredImport, ImportDiscoveryVisitor},
};

const FACTS_MODULE_ID: ModuleId = ModuleId::new(u32::MAX);
const FACTS_MODULE_NAME: &str = "__cribo_module_facts__";

/// Canonical per-module facts built once from the AST and reused across phases.
#[derive(Clone, Debug)]
pub(crate) struct ModuleFacts {
    /// Imports discovered with execution-context metadata.
    pub discovered_imports: Vec<DiscoveredImport>,
    /// Fine-grained module items consumed by the dependency graph and tree-shaking.
    ///
    /// These remain cached on `ModuleFacts` so later graph population clones each item into the
    /// destination `ModuleDepGraph` instead of transferring ownership out of the cached facts.
    pub items: Vec<ItemData>,
}

impl ModuleFacts {
    /// Build shared module facts from a parsed AST.
    pub(crate) fn from_ast(ast: &ModModule, python_version: u8) -> Result<Self> {
        Ok(Self {
            discovered_imports: Self::discover_imports(ast),
            items: Self::build_items(ast, python_version)?,
        })
    }

    /// Populate a `ModuleDepGraph` with the precomputed item metadata.
    ///
    /// Items are cloned here because cached `ModuleFacts` are reused across analyses, while each
    /// `ModuleDepGraph` needs owned `ItemData` values that it can index and mutate independently.
    pub(crate) fn populate_module_graph(&self, module: &mut ModuleDepGraph) {
        for item in &self.items {
            module.add_item(item.clone());
        }
    }

    fn discover_imports(ast: &ModModule) -> Vec<DiscoveredImport> {
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &ast.body {
            visitor.visit_stmt(stmt);
        }
        visitor.into_imports()
    }

    fn build_items(ast: &ModModule, python_version: u8) -> Result<Vec<ItemData>> {
        let mut module_graph = ModuleDepGraph::new(FACTS_MODULE_ID, FACTS_MODULE_NAME.to_owned());
        let mut builder = GraphBuilder::new(&mut module_graph, python_version);
        builder.build_from_ast(ast)?;
        Ok(module_graph.items.into_values().collect())
    }
}
