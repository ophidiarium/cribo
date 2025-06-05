# Circular Dependency Implementation Plan

Based on the research in `circular-dependencies-research.md` and analysis of existing test fixtures, this document outlines a comprehensive test-driven implementation approach for robust circular dependency handling in Serpen.

## Current State Analysis

### Existing Infrastructure ✅

- **Dependency Graph**: `DependencyGraph` with basic cycle detection via `has_cycles()`
- **Graph Library**: Using `petgraph` with DiGraph for module relationships
- **Test Infrastructure**: Robust snapshot testing with insta, comprehensive stickytape compatibility tests
- **Basic Detection**: Simple circular dependency detection available but **disabled** in tests

### Critical Gap Analysis ❌

- **Multi-module cycles**: Only 2-module cycle testing exists (and is disabled)
- **Resolution strategies**: No handling of detected cycles beyond error reporting
- **Complex circular patterns**: Missing package-level, relative import, and deep cycle scenarios
- **Tarjan's SCC Algorithm**: Not implemented for strongly connected component analysis
- **Error classification**: No differentiation between resolvable vs. unresolvable circular dependencies

## Implementation Strategy: Test-Driven Development Approach

### Phase 1: Comprehensive Test Suite Creation 🧪

#### 1.1 Multi-Module Circular Dependency Test Fixtures

**Priority: HIGH** - Create systematic test fixtures covering all circular dependency patterns:

```
tests/fixtures/circular_dependencies/
├── two_module_basic/           # ✅ Already exists (but disabled)
├── three_module_cycle/         # ❌ Missing
├── four_module_cycle/          # ❌ Missing  
├── complex_network/            # ❌ Missing
├── package_level_cycles/       # ❌ Missing
├── relative_import_cycles/     # ❌ Missing
├── mixed_import_style_cycles/  # ❌ Missing
├── unresolvable_patterns/      # ❌ Missing
└── edge_cases/                 # ❌ Missing
```

**Test Scenarios to Implement:**

1. **Three-Module Cycle**: `A → B → C → A`
   ```python
   # module_a.py
   from module_b import process_b
   def process_a(): return process_b() + "->A"

   # module_b.py  
   from module_c import process_c
   def process_b(): return process_c() + "->B"

   # module_c.py
   from module_a import process_a
   def process_c(): return "C"  # This would fail, but demonstrates pattern
   ```

2. **Package-Level Circular Dependencies**:
   ```python
   # pkg1/__init__.py
   from pkg2 import helper

   # pkg2/__init__.py
   from pkg1 import main_func
   ```

3. **Relative Import Cycles**:
   ```python
   # services/auth.py
   from .database import get_connection

   # services/database.py
   from .auth import get_user_context
   ```

4. **Unresolvable Patterns** (should fail gracefully):
   ```python
   # constants_a.py
   from constants_b import B_VALUE
   A_VALUE = B_VALUE + 1

   # constants_b.py
   from constants_a import A_VALUE  
   B_VALUE = A_VALUE * 2  # Temporal paradox - unresolvable
   ```

#### 1.2 Expected Behavior Specification

**Test Categories:**

1. **Resolvable Cycles**: Should bundle successfully with proper module ordering
2. **Unresolvable Cycles**: Should fail with detailed diagnostic messages
3. **Partial Resolution**: Should bundle non-circular parts and report problematic cycles
4. **Performance**: Large cycle networks should complete within reasonable time bounds

### Phase 2: Enhanced Dependency Graph Implementation 🔧

#### 2.1 Implement Tarjan's Strongly Connected Components Algorithm

**File**: `crates/serpen/src/dependency_graph.rs`

**New Methods to Add:**

```rust
impl DependencyGraph {
    /// Find all strongly connected components (circular dependency groups)
    pub fn find_strongly_connected_components(&self) -> Vec<Vec<NodeIndex>> {
        // Implement Tarjan's algorithm
        // Returns groups of modules that are circularly dependent
    }

    /// Get detailed cycle information for diagnostics
    pub fn find_cycle_paths(&self) -> Result<Vec<Vec<String>>> {
        // Use DFS with three-color marking to find exact cycle paths
        // Essential for error reporting
    }

    /// Classify circular dependencies by type
    pub fn classify_circular_dependencies(&self) -> CircularDependencyAnalysis {
        // Categorize cycles as:
        // - Function-level (resolvable)
        // - Class-level (potentially resolvable)
        // - Module-level constants (unresolvable)
        // - Import-time dependencies (context-dependent)
    }

    /// Suggest resolution strategies for detected cycles
    pub fn suggest_resolution_strategies(&self, cycles: &[Vec<String>]) -> Vec<ResolutionStrategy> {
        // Provide actionable suggestions:
        // - Move imports inside functions
        // - Use lazy imports
        // - Refactor to remove circular dependency
        // - Split modules
    }
}
```

#### 2.2 New Data Structures

```rust
#[derive(Debug, Clone)]
pub struct CircularDependencyAnalysis {
    pub resolvable_cycles: Vec<CircularDependencyGroup>,
    pub unresolvable_cycles: Vec<CircularDependencyGroup>,
    pub total_cycles_detected: usize,
    pub largest_cycle_size: usize,
}

#[derive(Debug, Clone)]
pub struct CircularDependencyGroup {
    pub modules: Vec<String>,
    pub cycle_type: CircularDependencyType,
    pub import_chain: Vec<ImportEdge>,
    pub suggested_resolution: ResolutionStrategy,
}

#[derive(Debug, Clone)]
pub enum CircularDependencyType {
    FunctionLevel,   // Can be resolved by moving imports inside functions
    ClassLevel,      // May be resolvable depending on usage patterns
    ModuleConstants, // Unresolvable - temporal paradox
    ImportTime,      // Depends on execution order
}

#[derive(Debug, Clone)]
pub enum ResolutionStrategy {
    LazyImport { modules: Vec<String> },
    FunctionScopedImport { import_statements: Vec<String> },
    ModuleSplit { suggestions: Vec<String> },
    Unresolvable { reason: String },
}

#[derive(Debug, Clone)]
pub struct ImportEdge {
    pub from_module: String,
    pub to_module: String,
    pub import_type: ImportType,
    pub line_number: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum ImportType {
    Direct,         // import module
    FromImport,     // from module import item
    RelativeImport, // from .module import item
    AliasedImport,  // import module as alias
}
```

### Phase 3: Circular Dependency Resolution Engine 🛠️

#### 3.1 Multi-Phase Resolution Strategy

**File**: `crates/serpen/src/circular_dependency_resolver.rs` (new)

```rust
pub struct CircularDependencyResolver {
    graph: DependencyGraph,
    resolution_config: ResolutionConfig,
}

impl CircularDependencyResolver {
    /// Phase 1: Detect and analyze all circular dependencies
    pub fn analyze_cycles(&mut self) -> Result<CircularDependencyAnalysis> {
        // 1. Run Tarjan's algorithm to find SCCs
        // 2. Classify each cycle by type
        // 3. Determine resolution strategy for each cycle
    }

    /// Phase 2: Apply resolution strategies
    pub fn resolve_cycles(
        &mut self,
        analysis: &CircularDependencyAnalysis,
    ) -> Result<ResolvedGraph> {
        // 1. Group resolvable cycles into bundle units
        // 2. Generate modified import statements for lazy loading
        // 3. Create initialization order for circular groups
        // 4. Generate error reports for unresolvable cycles
    }

    /// Phase 3: Generate bundle with resolved circular dependencies
    pub fn generate_bundle(&self, resolved: &ResolvedGraph) -> Result<BundleOutput> {
        // 1. Apply Kahn's algorithm for final module ordering
        // 2. Generate bundle with proper initialization sequence
        // 3. Include runtime circular dependency helpers if needed
    }
}
```

#### 3.2 Resolution Strategies Implementation

1. **Lazy Import Strategy**: Convert problematic imports to function-scoped imports
2. **Bundle Grouping**: Combine circular modules into single bundle units
3. **Initialization Ordering**: Determine safe execution order for circular groups
4. **Runtime Helpers**: Generate proxy objects for complex circular patterns

### Phase 4: Enhanced Error Reporting & Diagnostics 📊

#### 4.1 Detailed Cycle Path Reporting

```rust
pub struct CircularDependencyError {
    pub cycle_path: Vec<String>,
    pub cycle_type: CircularDependencyType,
    pub resolution_suggestions: Vec<String>,
    pub affected_imports: Vec<ImportLocation>,
    pub error_severity: ErrorSeverity,
}

#[derive(Debug)]
pub enum ErrorSeverity {
    Warning, // Resolvable cycle detected
    Error,   // Unresolvable cycle blocks bundling
    Info,    // Cycle resolved automatically
}
```

#### 4.2 Actionable Error Messages

Transform generic "circular dependency detected" into:

```
Error: Unresolvable circular dependency detected

Cycle path: module_a.py → module_b.py → module_a.py
Issue: Module-level constant dependency creates temporal paradox

  module_a.py:3  | A_VALUE = B_VALUE + 1
  module_b.py:3  | B_VALUE = A_VALUE * 2

Suggested resolution:
  1. Move one of the constants to a separate configuration module
  2. Use dependency injection to break the circular reference
  3. Compute values at runtime rather than import-time

For more information: https://serpen.dev/docs/circular-dependencies
```

### Phase 5: Integration & Performance Optimization 🚀

#### 5.1 Integration Points

1. **Bundler Integration**: Update main bundler to use new circular dependency resolver
2. **CLI Integration**: Add flags for circular dependency handling behavior
3. **Configuration**: Allow users to configure resolution strategies via `serpen.toml`

#### 5.2 Performance Considerations

1. **Incremental Analysis**: Cache circular dependency analysis results
2. **Large Graph Optimization**: Use approximation algorithms for massive dependency networks
3. **Memory Efficiency**: Stream processing for large codebases

## Implementation Timeline & Test-Driven Milestones

### Sprint 1: Foundation (Tests + Basic Detection)

- [ ] Create comprehensive test fixture suite
- [ ] Implement Tarjan's SCC algorithm
- [ ] Add cycle path detection with three-color DFS
- [ ] Enable and fix existing circular dependency tests

### Sprint 2: Classification & Analysis

- [ ] Implement circular dependency type classification
- [ ] Add detailed import analysis
- [ ] Create resolution strategy suggestion engine
- [ ] Comprehensive error reporting system

### Sprint 3: Resolution Engine

- [ ] Implement lazy import transformation
- [ ] Add bundle grouping for circular modules
- [ ] Create initialization ordering system
- [ ] Generate runtime circular dependency helpers

### Sprint 4: Integration & Polish

- [ ] Integrate with main bundler pipeline
- [ ] Add CLI flags and configuration options
- [ ] Performance optimization for large graphs
- [ ] Documentation and examples

## Success Criteria

### Functional Requirements ✅

1. **Detection**: Identify all types of circular dependencies accurately
2. **Classification**: Differentiate between resolvable and unresolvable cycles
3. **Resolution**: Successfully bundle 85%+ of real-world circular dependency scenarios
4. **Reporting**: Provide actionable error messages with specific resolution suggestions

### Performance Requirements ✅

1. **Speed**: Analyze dependency graphs of 1000+ modules in under 5 seconds
2. **Memory**: Scale to large codebases without excessive memory usage
3. **Accuracy**: Zero false positives for cycle detection

### Quality Requirements ✅

1. **Test Coverage**: 95%+ coverage for all circular dependency handling code
2. **Regression Protection**: Comprehensive snapshot tests for all resolution strategies
3. **Error Handling**: Graceful degradation when resolution fails
4. **Documentation**: Clear examples and troubleshooting guides

## Risk Mitigation

### Technical Risks

1. **Complex Cycle Networks**: Use approximation algorithms for NP-hard scenarios
2. **Performance**: Implement incremental analysis and caching
3. **False Positives**: Extensive test coverage with real-world codebases

### Implementation Risks

1. **Scope Creep**: Phase-gated approach with clear milestones
2. **Breaking Changes**: Maintain backward compatibility with configuration flags
3. **Edge Cases**: Comprehensive test fixture coverage before implementation

## Future Enhancements

### Phase 6: Advanced Features (Future)

- **Machine Learning**: Learn resolution patterns from user behavior
- **IDE Integration**: Real-time circular dependency warnings
- **Refactoring Suggestions**: Automated code restructuring recommendations
- **Ecosystem Integration**: Integration with popular Python tools (mypy, pylint, etc.)

---

This implementation plan provides a systematic, test-driven approach to implementing robust circular dependency handling in Serpen, based on proven algorithms and real-world requirements analysis.
