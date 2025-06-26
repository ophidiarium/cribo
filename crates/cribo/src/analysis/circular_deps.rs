//! Circular dependency analysis with structured types
//!
//! This module provides analysis for circular dependencies using ModuleId
//! instead of string-based references for better type safety and consistency.

use rustc_hash::FxHashMap;

use crate::cribo_graph::{ItemId, ModuleId};

/// An edge between modules in the dependency graph
#[derive(Debug, Clone)]
pub struct ModuleEdge {
    /// Source module
    pub from_module: ModuleId,
    /// Target module
    pub to_module: ModuleId,
    /// Type of import relationship
    pub edge_type: EdgeType,
    /// Metadata about the import
    pub metadata: EdgeMetadata,
}

/// Type of edge between modules
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeType {
    /// Direct import: `import module`
    DirectImport,
    /// From import: `from module import item`
    FromImport { symbols: Vec<String> },
    /// Relative import: `from .module import item`
    RelativeImport { level: u32, symbols: Vec<String> },
    /// Aliased import: `import module as alias`
    AliasedImport { alias: String },
}

/// Metadata about an import edge
#[derive(Debug, Clone)]
pub struct EdgeMetadata {
    /// Line number where import occurs
    pub line_number: Option<usize>,
    /// The ItemId of the import statement
    pub import_item_id: Option<ItemId>,
    /// Whether this import is at module level
    pub is_module_level: bool,
    /// Function name if import is inside a function
    pub containing_function: Option<String>,
}

/// Comprehensive analysis of circular dependencies
#[derive(Debug, Clone)]
pub struct CircularDependencyAnalysis {
    /// Circular dependencies that can be resolved through code transformations
    pub resolvable_cycles: Vec<CircularDependencyGroup>,
    /// Circular dependencies that cannot be resolved
    pub unresolvable_cycles: Vec<CircularDependencyGroup>,
    /// Total number of cycles detected
    pub total_cycles_detected: usize,
    /// Size of the largest cycle
    pub largest_cycle_size: usize,
    /// All cycle paths found (using ModuleIds)
    pub cycle_paths: Vec<Vec<ModuleId>>,
}

/// A group of modules forming a circular dependency
#[derive(Debug, Clone)]
pub struct CircularDependencyGroup {
    /// Modules in the cycle
    pub module_ids: Vec<ModuleId>,
    /// Type of circular dependency
    pub cycle_type: CircularDependencyType,
    /// Import chain forming the cycle
    pub import_chain: Vec<ModuleEdge>,
    /// Suggested resolution strategy
    pub suggested_resolution: ResolutionStrategy,
    /// Analysis metadata
    pub metadata: CycleMetadata,
}

/// Additional metadata about a cycle
#[derive(Debug, Clone)]
pub struct CycleMetadata {
    /// Whether all imports in cycle are function-scoped
    pub all_function_scoped: bool,
    /// Whether cycle involves any class definitions
    pub involves_classes: bool,
    /// Whether cycle has module-level constants
    pub has_module_constants: bool,
    /// Complexity score for resolution priority
    pub complexity_score: u32,
}

/// Type of circular dependency
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CircularDependencyType {
    /// Can be resolved by moving imports inside functions
    FunctionLevel,
    /// May be resolvable depending on usage patterns
    ClassLevel,
    /// Unresolvable - temporal paradox with module constants
    ModuleConstants,
    /// Depends on execution order at import time
    ImportTime,
}

/// Resolution strategy for circular dependencies
#[derive(Debug, Clone)]
pub enum ResolutionStrategy {
    /// Use lazy import pattern
    LazyImport {
        /// Modules to make lazy
        module_ids: Vec<ModuleId>,
        /// Suggested lazy variable names
        lazy_var_names: FxHashMap<ModuleId, String>,
    },
    /// Move imports to function scope
    FunctionScopedImport {
        /// Map of import ItemId to target function ItemId
        import_to_function: FxHashMap<ItemId, ItemId>,
        /// Human-readable description of moves
        descriptions: Vec<String>,
    },
    /// Split module to break cycle
    ModuleSplit {
        /// Module to split
        module_id: ModuleId,
        /// Suggested new module names
        suggested_names: Vec<String>,
        /// Items to move to each new module
        item_distribution: Vec<Vec<ItemId>>,
    },
    /// Cannot be resolved automatically
    Unresolvable {
        /// Reason why it cannot be resolved
        reason: String,
        /// Manual intervention suggestions
        manual_suggestions: Vec<String>,
    },
}

impl CircularDependencyAnalysis {
    /// Create a new empty analysis result
    pub fn new() -> Self {
        Self {
            resolvable_cycles: Vec::new(),
            unresolvable_cycles: Vec::new(),
            total_cycles_detected: 0,
            largest_cycle_size: 0,
            cycle_paths: Vec::new(),
        }
    }

    /// Check if any cycles were detected
    pub fn has_cycles(&self) -> bool {
        self.total_cycles_detected > 0
    }

    /// Get all cycles (both resolvable and unresolvable)
    pub fn all_cycles(&self) -> Vec<&CircularDependencyGroup> {
        self.resolvable_cycles
            .iter()
            .chain(self.unresolvable_cycles.iter())
            .collect()
    }

    /// Add a resolvable cycle
    pub fn add_resolvable_cycle(&mut self, cycle: CircularDependencyGroup) {
        self.total_cycles_detected += 1;
        self.largest_cycle_size = self.largest_cycle_size.max(cycle.module_ids.len());
        self.cycle_paths.push(cycle.module_ids.clone());
        self.resolvable_cycles.push(cycle);
    }

    /// Add an unresolvable cycle
    pub fn add_unresolvable_cycle(&mut self, cycle: CircularDependencyGroup) {
        self.total_cycles_detected += 1;
        self.largest_cycle_size = self.largest_cycle_size.max(cycle.module_ids.len());
        self.cycle_paths.push(cycle.module_ids.clone());
        self.unresolvable_cycles.push(cycle);
    }
}

impl Default for CircularDependencyAnalysis {
    fn default() -> Self {
        Self::new()
    }
}
