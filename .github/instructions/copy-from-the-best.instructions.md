---
applyTo: "**"
---

# Reference Patterns from Established Repositories

When implementing functionality or uncertain about design patterns, library usage, or best practices, consult these high-quality repositories for proven approaches and patterns.

## Primary Reference Repositories

### Python/Rust Tooling Patterns

When working with Python analysis, AST manipulation, configuration handling, or general Rust patterns:

1. **[astral-sh/ruff](https://github.com/astral-sh/ruff)** - Modern Python linter/formatter in Rust
   - **Use for**: Python AST handling, rule implementation, configuration patterns, CLI design, error reporting
   - **Key areas**: `crates/ruff_python_*`, `crates/ruff_linter/`, configuration handling

2. **[astral-sh/uv](https://github.com/astral-sh/uv)** - Fast Python package installer in Rust
   - **Use for**: Package resolution, dependency management, virtual environment handling, Python ecosystem integration
   - **Key areas**: Dependency resolution algorithms, package metadata handling, cross-platform compatibility

3. **[facebook/pyrefly](https://github.com/facebook/pyrefly)** - High-performance Python static analyzer
   - **Use for**: Type inference, incremental analysis, large codebase handling, Python semantics
   - **Key areas**: Type system implementation, incremental checking, performance optimization

4. **[quarylabs/sqruff](https://github.com/quarylabs/sqruff)** - SQL linter and formatter with advanced lineage analysis
   - **Use for**: SQL parsing and formatting, lineage analysis, test inference algorithms, benchmarking setup, modular crate architecture
   - **Key areas**: `crates/lineage/`, `crates/lib-core/`, benchmarking with Criterion, SQL AST manipulation

### Bundling and Dependency Graph Patterns

When implementing bundling logic, dependency resolution, tree shaking, or module analysis:

1. **[web-infra-dev/rspack](https://github.com/web-infra-dev/rspack)** - Fast Rust-based bundler
   - **Use for**: Module graph construction, dependency resolution, plugin systems, parallel processing
   - **Key areas**: `crates/rspack_core/`, dependency graph algorithms, module federation

2. **[umijs/mako](https://github.com/umijs/mako)** - Modern Rust-based bundler algorithms reference
   - **Use for**: Efficient bundling algorithm implementations, tree shaking strategies, plugin architectures
   - **Key areas**: core bundler algorithms, module resolution, optimization passes

## Implementation Guidelines

### 1. Research Before Implementation

**GitHub MCP Server Tools Available:**

- `github_repo` - Search specific repositories for code snippets (use this for targeted searches)
- `f1e_search_code` - Search code across all GitHub repositories
- `f1e_get_file_contents` - Retrieve specific files from reference repositories
- `f1e_search_repositories` - Find additional high-quality reference repositories

**Example Research Process:**

```bash
# Example: If implementing dependency resolution
# 1. Use github_repo to search "astral-sh/uv" for dependency resolution patterns
# 2. Use github_repo to search "web-infra-dev/rspack" for module graph construction  
# 3. Use github_repo to search "umijs/mako" for efficient algorithms
# 4. Use f1e_get_file_contents to examine specific implementation files
```

**Process:**

1. Identify the core functionality you're implementing
2. Determine which repositories are most relevant
3. Use GitHub MCP tools to search for similar implementations in those repositories
4. Adapt their patterns to your specific use case
5. Document the source of inspiration in code comments

### 2. Specific Use Cases

#### Configuration Handling

- **Reference**: `ruff` configuration system
- **Pattern**: TOML-based config with validation, hierarchical resolution
- **Files to check**: `crates/ruff_workspace/src/configuration.rs`

#### Error Reporting and Diagnostics

- **Reference**: `ruff` diagnostic system
- **Pattern**: Rich error messages with source location, suggestions for fixes
- **Files to check**: `crates/ruff_diagnostics/`

#### CLI Design

- **Reference**: `ruff` and `uv` CLI interfaces
- **Pattern**: Clap-based subcommands, consistent help text, progress reporting
- **Files to check**: `crates/ruff_cli/`, `crates/uv/src/main.rs`

#### Dependency Resolution

- **Reference**: `uv` resolver implementation
- **Pattern**: SAT-based resolution, conflict handling, version constraints
- **Files to check**: `crates/uv-resolver/`

#### Module Graph Construction

- **Reference**: `rspack` module graph
- **Pattern**: Efficient graph representation, cycle detection, incremental updates
- **Files to check**: `crates/rspack_core/src/module_graph/`

#### Tree Shaking and Dead Code Elimination

- **Reference**: `mako` tree shaking
- **Pattern**: Usage analysis, side-effect tracking, optimization passes
- **Files to check**: core bundler algorithms, optimization passes

#### SQL Analysis and Lineage Tracking

- **Reference**: `sqruff` lineage analysis
- **Pattern**: SQL AST traversal, dependency graph construction, column-level lineage tracking
- **Files to check**: `crates/lineage/src/`, scope analysis, IR construction

#### Advanced Benchmarking Setup

- **Reference**: `sqruff` benchmarking infrastructure
- **Pattern**: Criterion-based performance testing, profiling integration, flamegraph generation
- **Files to check**: Benchmarking setup with PProfProfiler, performance regression detection

#### AST Manipulation and Transformation

- **Reference**: `ruff` Python AST handling
- **Pattern**: Visitor patterns, safe transformations, preserving semantics
- **Files to check**: `crates/ruff_python_ast/`

#### Lineage Analysis and Data Flow Tracking

- **Reference**: `sqruff` lineage implementation
- **Pattern**: Multi-pass analysis (lowering → expansion → qualification → scope building), intermediate representation design
- **Files to check**: `crates/lineage/src/ir.rs`, `crates/lineage/src/scope.rs`, `crates/lineage/src/qualify.rs`
- **Key techniques**:
  - Scope-based traversal with hierarchical source tracking
  - Expression graph construction with parent-child relationships
  - Column-level dependency resolution through table and CTE analysis
  - Union and subquery handling with proper scope isolation

### 3. Code Documentation Standards

When adapting patterns from these repositories:

```rust
/// Implements dependency resolution using SAT-based approach
/// Inspired by uv's resolver: https://github.com/astral-sh/uv/blob/main/crates/uv-resolver/
/// Key improvements: [describe your adaptations]
pub struct DependencyResolver {
    // Implementation details
}

/// Implements column-level lineage tracking using scope-based analysis
/// Inspired by sqruff's lineage engine: https://github.com/quarylabs/sqruff/tree/main/crates/lineage/
/// Adapted for [your specific domain] with improvements in [describe adaptations]
pub struct LineageTracker {
    // Implementation based on sqruff's ir.rs and scope.rs patterns
}
```

**Required documentation:**

- Link to the source repository and specific files
- Brief description of the pattern being used
- Any adaptations or improvements made for your use case
- Why this approach was chosen over alternatives

### 4. Testing Patterns

Follow testing approaches from reference repositories:

- **Unit tests**: Copy comprehensive test coverage patterns from `ruff`
- **Integration tests**: Follow `uv`'s approach to testing complex workflows
- **Benchmarks**: Use `mako`'s benchmarking patterns for performance validation
- **Property-based tests**: Adopt `rspack`'s fuzzing and property testing
- **Lineage validation**: Follow `sqruff`'s approach to testing complex data flow scenarios

### 5. Performance Optimization

Reference performance patterns:

- **Parallel processing**: `rspack`'s parallel compilation strategies
- **Memory efficiency**: `mako`'s memory management techniques
- **Incremental compilation**: `ruff`'s incremental analysis patterns
- **Caching strategies**: `uv`'s caching mechanisms
- **Graph traversal optimization**: `sqruff`'s efficient scope-based analysis

## Decision Making Process

When facing design decisions:

1. **Search for precedent** in the reference repositories
2. **Compare approaches** across multiple repositories
3. **Consider your specific constraints** (performance, maintainability, compatibility)
4. **Document the decision** with references to source patterns
5. **Test thoroughly** using patterns from reference implementations

## Repository-Specific Strengths

### Use Ruff for:

- Python language understanding and AST manipulation
- Configuration system design
- Error reporting and user experience
- Incremental analysis patterns

### Use uv for:

- Dependency resolution algorithms
- Package ecosystem integration
- Cross-platform compatibility
- Performance optimization in Python tooling

### Use Pyrefly for:

- Type system implementation
- Static analysis techniques
- Handling large codebases efficiently
- Python semantic understanding

### Use Rspack for:

- Module graph construction and manipulation
- Plugin architecture design
- Parallel processing strategies
- Webpack compatibility patterns

### Use Mako for:

- Efficient bundling algorithms
- Tree shaking strategies
- Plugin architecture design
- Optimization passes

### Use Sqruff for:

- SQL parsing and AST manipulation
- Advanced lineage analysis and dependency tracking
- Column-level data flow analysis
- Modular crate architecture patterns
- Comprehensive benchmarking and profiling setup
- Test inference algorithms for data validation
- Schema evolution and migration patterns

## Maintenance

- **Regular updates**: Monitor these repositories for new patterns and improvements
- **Version tracking**: Note which version/commit of reference patterns you're using
- **Migration planning**: Plan updates when reference repositories evolve their patterns
- **Community engagement**: Contribute back improvements or report issues to reference repositories

Remember: The goal is not to copy blindly, but to understand proven patterns and adapt them thoughtfully to your specific requirements while maintaining the quality and reliability demonstrated by these established projects.
