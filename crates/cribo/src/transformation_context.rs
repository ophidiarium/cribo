//! Transformation context for tracking AST transformations and node mappings.
//!
//! This module provides a context that tracks how AST nodes are transformed
//! during the bundling process, enabling future source map generation.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use ruff_python_ast::{AtomicNodeIndex, NodeIndex};

/// Context for tracking transformations during bundling
#[derive(Debug)]
pub struct TransformationContext {
    /// Counter for assigning new node indices
    next_index: AtomicU32,
    /// Track which transformations were applied
    pub transformations: Vec<TransformationRecord>,
}

/// Record of a transformation applied to a node
#[derive(Debug, Clone)]
pub struct TransformationRecord {
    /// Original module and node
    pub original: Option<(Arc<Path>, NodeIndex)>,
    /// Transformed node index
    pub transformed: NodeIndex,
    /// Type of transformation applied
    pub transformation_type: TransformationType,
}

/// Types of transformations that can be applied to nodes
#[derive(Debug, Clone, PartialEq)]
pub enum TransformationType {
    /// Node was copied directly without changes
    DirectCopy,
    /// Import statement was rewritten
    ImportRewritten {
        from_module: String,
        to_module: String,
    },
    /// globals() call was replaced with module.__dict__
    GlobalsReplaced,
    /// Module was wrapped in sys.modules registration
    ModuleWrapped { module_name: String },
    /// Node was eliminated as dead code
    DeadCodeEliminated,
    /// New node created during transformation
    NewNode { reason: String },
    /// Multiple nodes merged into one
    NodesMerged { source_count: usize },
}

impl TransformationContext {
    /// Create a new transformation context
    pub fn new() -> Self {
        Self {
            next_index: AtomicU32::new(0),
            transformations: Vec::new(),
        }
    }

    /// Get the next available node index
    pub fn next_node_index(&self) -> u32 {
        self.next_index.fetch_add(1, Ordering::Relaxed)
    }

    /// Create a new node with a fresh index
    pub fn create_node_index(&self) -> AtomicNodeIndex {
        let index = self.next_node_index();
        AtomicNodeIndex::from(index)
    }

    /// Record a direct copy transformation
    pub fn record_copy(
        &mut self,
        original_module: Arc<Path>,
        original_node: NodeIndex,
        transformed_node: &AtomicNodeIndex,
    ) -> NodeIndex {
        let transformed_index = AtomicNodeIndex::from(self.next_node_index()).load();
        transformed_node.set(transformed_index.as_usize() as u32);

        self.transformations.push(TransformationRecord {
            original: Some((original_module, original_node)),
            transformed: transformed_index,
            transformation_type: TransformationType::DirectCopy,
        });

        transformed_index
    }

    /// Record a transformation with a specific type
    pub fn record_transformation(
        &mut self,
        original: Option<(Arc<Path>, NodeIndex)>,
        transformed_node: &AtomicNodeIndex,
        transformation_type: TransformationType,
    ) -> NodeIndex {
        let transformed_index = AtomicNodeIndex::from(self.next_node_index()).load();
        transformed_node.set(transformed_index.as_usize() as u32);

        self.transformations.push(TransformationRecord {
            original,
            transformed: transformed_index,
            transformation_type,
        });

        transformed_index
    }

    /// Create a completely new node
    pub fn create_new_node(&mut self, reason: String) -> AtomicNodeIndex {
        let node_index = self.create_node_index();
        let index = node_index.load();

        self.transformations.push(TransformationRecord {
            original: None,
            transformed: index,
            transformation_type: TransformationType::NewNode { reason },
        });

        node_index
    }

    /// Get transformation info for a given transformed node
    pub fn get_transformation(&self, node_index: NodeIndex) -> Option<&TransformationRecord> {
        self.transformations
            .iter()
            .find(|t| t.transformed == node_index)
    }

    /// Get statistics about transformations
    pub fn get_stats(&self) -> TransformationStats {
        let mut stats = TransformationStats::default();

        for transformation in &self.transformations {
            match &transformation.transformation_type {
                TransformationType::DirectCopy => stats.direct_copies += 1,
                TransformationType::ImportRewritten { .. } => stats.imports_rewritten += 1,
                TransformationType::GlobalsReplaced => stats.globals_replaced += 1,
                TransformationType::ModuleWrapped { .. } => stats.modules_wrapped += 1,
                TransformationType::DeadCodeEliminated => stats.dead_code_eliminated += 1,
                TransformationType::NewNode { .. } => stats.new_nodes += 1,
                TransformationType::NodesMerged { .. } => stats.nodes_merged += 1,
            }
        }

        stats.total_transformations = self.transformations.len();
        stats
    }
}

impl Default for TransformationContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about transformations applied
#[derive(Debug, Default)]
pub struct TransformationStats {
    pub total_transformations: usize,
    pub direct_copies: usize,
    pub imports_rewritten: usize,
    pub globals_replaced: usize,
    pub modules_wrapped: usize,
    pub dead_code_eliminated: usize,
    pub new_nodes: usize,
    pub nodes_merged: usize,
}
