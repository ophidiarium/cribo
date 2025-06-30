//! Transformation context for tracking AST transformations and node mappings.
//!
//! This module provides a context that tracks how AST nodes are transformed
//! during the bundling process, enabling future source map generation.

use std::sync::atomic::{AtomicU32, Ordering};

use ruff_python_ast::AtomicNodeIndex;

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
    /// Type of transformation applied
    pub transformation_type: TransformationType,
}

/// Types of transformations that can be applied to nodes
#[derive(Debug, Clone, PartialEq)]
pub enum TransformationType {
    /// New node created during transformation
    NewNode { reason: String },
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

    /// Create a completely new node
    pub fn create_new_node(&mut self, reason: String) -> AtomicNodeIndex {
        let node_index = self.create_node_index();
        self.transformations.push(TransformationRecord {
            transformation_type: TransformationType::NewNode { reason },
        });

        node_index
    }

    /// Get statistics about transformations
    pub fn get_stats(&self) -> TransformationStats {
        let mut stats = TransformationStats::default();

        for transformation in &self.transformations {
            match &transformation.transformation_type {
                TransformationType::NewNode { .. } => stats.new_nodes += 1,
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
    pub new_nodes: usize,
}
