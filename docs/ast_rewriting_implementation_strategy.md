# AST Rewriting Implementation Strategy for Serpen Python Bundler

## Executive Summary

This document analyzes how JavaScript bundlers perform AST rewriting and adapts these approaches for implementing a reliable Python AST unparser for the Serpen bundler. The goal is to enable bundling Python modules into a single file using AST rewriting techniques (similar to JavaScript bundlers like webpack, rollup, and rspack) rather than text concatenation, while fixing current Int conversion errors and integrating comprehensive unparsing capabilities.

## JavaScript Bundler AST Rewriting Analysis

### Core Patterns Across Modern Bundlers

Through analysis of rspack, rollup, and mako bundlers, several consistent patterns emerge:

#### 1. Module Graph Construction

- **Dependency Discovery**: Bundlers parse entry points and recursively discover dependencies through import/export statements
- **Graph Representation**: Module dependencies are represented as directed graphs with nodes (modules) and edges (import relationships)
- **Metadata Tracking**: Each module node stores metadata including exports, imports, used identifiers, and AST representations

#### 2. AST Transformation Pipeline

```
Module Loading → AST Parsing → Dependency Analysis → AST Transformation → Code Generation
```

**Key Transformation Steps:**

- **Import/Export Rewriting**: Transform ES6 imports/exports into runtime calls or direct variable references
- **Scope Analysis**: Track variable scopes to prevent naming conflicts during module concatenation
- **Tree Shaking**: Remove unused exports and their dependencies through used identifier analysis
- **Module Concatenation**: Combine multiple modules into single scope with proper variable renaming

#### 3. Visitor Pattern Implementation

Bundlers use AST visitor patterns for systematic tree traversal and transformation:

```rust
// Rspack-style visitor pattern
impl Visit for ConcatenatedTransform {
    fn visit_import_declaration(&mut self, node: &ImportDeclaration) {
        // Transform import into module reference
        self.rewrite_import(node);
    }

    fn visit_export_declaration(&mut self, node: &ExportDeclaration) {
        // Transform export into variable assignment
        self.rewrite_export(node);
    }
}
```

#### 4. Tree Shaking Mechanisms

- **Used Identifier Tracking**: Track which exports are actually imported and used
- **Statement-Level Analysis**: Analyze dependencies at statement level for precise elimination
- **Side Effect Detection**: Preserve statements with side effects even if their exports aren't used

### Specific Bundler Approaches

#### Rspack AST Rewriting

- Uses `ConcatenatedTransform` visitor for import/export rewriting
- Implements module graph with `ModuleGraph` structure tracking dependencies
- Performs tree shaking through `UsedIdentifierTracker`
- Handles ES6 modules by converting to CommonJS-style runtime calls

#### Rollup Module Processing

- Implements `ModuleLoader` for dependency resolution and AST parsing
- Uses `MagicString` for efficient code transformations with source maps
- Performs tree shaking through export usage analysis
- Concatenates modules with scope hoisting to eliminate module boundaries

#### Mako Tree Shaking

- Implements statement-level dependency tracking
- Uses visitor pattern for AST transformation: `visit_import_decl`, `visit_export_decl`
- Handles external modules through `is_external_module` checks
- Performs module concatenation with proper scope management

## Python-Specific Challenges and Considerations

### Import System Differences

- **Relative vs Absolute Imports**: Python supports both `from .module import x` and `from module import x`
- **Import Levels**: Python's `from ...parent import x` has no JavaScript equivalent
- **Dynamic Imports**: `__import__()` and `importlib` require runtime analysis
- **Namespace Packages**: Python's namespace packages have complex resolution rules

### AST Node Differences

- **Module Structure**: Python modules are statements, not expressions like JavaScript
- **Import Variants**: `import x`, `from x import y`, `from x import y as z`, `import x as y`
- **Global/Nonlocal**: Python's scoping keywords need special handling during concatenation
- **Decorators**: Python decorators can modify class/function behavior and need preservation

### Execution Model Differences

- **Module Initialization**: Python modules execute top-level code on import
- **Circular Imports**: Python's import system handles circular dependencies differently
- **Import Order**: Python import order can affect behavior through side effects

## Implementation Strategy for Serpen

### Phase 1: Foundation - AST Infrastructure

#### 1.1 Dependency Integration

Add `rustpython-unparser` as external dependency:

```toml
# Cargo.toml
[dependencies]
rustpython-unparser = "0.3"
rustpython-parser = "0.3"
```

#### 1.2 Core Data Structures

Implement module graph and transformation infrastructure:

```rust
// Module graph representation
pub struct ModuleGraph {
    modules: HashMap<ModuleId, ModuleNode>,
    dependencies: HashMap<ModuleId, Vec<Dependency>>,
    entry_points: Vec<ModuleId>,
}

pub struct ModuleNode {
    id: ModuleId,
    path: PathBuf,
    ast: rustpython_parser::ast::Suite,
    imports: Vec<ImportInfo>,
    exports: Vec<ExportInfo>,
    used_names: HashSet<String>,
}

pub struct ImportInfo {
    module: String,
    names: Vec<String>,
    level: u32, // For relative imports
    alias: Option<String>,
}
```

#### 1.3 AST Visitor Framework

Implement visitor pattern for Python AST transformation:

```rust
pub trait PyAstVisitor {
    fn visit_import(&mut self, node: &Import) -> Import;
    fn visit_import_from(&mut self, node: &ImportFrom) -> Option<Stmt>;
    fn visit_function_def(&mut self, node: &FunctionDef) -> FunctionDef;
    fn visit_class_def(&mut self, node: &ClassDef) -> ClassDef;
}

pub struct BundleTransformer {
    module_map: HashMap<String, ModuleId>,
    name_mangler: NameMangler,
    used_imports: HashSet<String>,
}
```

### Phase 2: Module Discovery and Analysis

#### 2.1 Dependency Resolution

Implement Python-aware module resolution:

```rust
impl ModuleResolver {
    fn resolve_import(&self, import: &ImportFrom, current_module: &Path) -> Option<PathBuf> {
        match import.level {
            0 => self.resolve_absolute(import),
            n => self.resolve_relative(import, current_module, n),
        }
    }

    fn build_module_graph(&self, entry_point: &Path) -> Result<ModuleGraph> {
        // Recursively discover and parse all dependencies
        // Build dependency graph with proper Python import semantics
    }
}
```

#### 2.2 Import Analysis

Track all import patterns and their usage:

```rust
impl ImportAnalyzer {
    fn analyze_imports(&self, ast: &Suite) -> Vec<ImportInfo> {
        // Extract all import statements
        // Track imported names and their usage
        // Handle star imports specially
    }

    fn find_used_names(&self, ast: &Suite) -> HashSet<String> {
        // Analyze AST to find which imported names are actually used
        // Track through function calls, attribute access, etc.
    }
}
```

### Phase 3: AST Transformation

#### 3.1 Import/Export Rewriting

Transform Python imports for bundling:

```rust
impl BundleTransformer {
    fn transform_import(&mut self, import: &Import) -> Option<Stmt> {
        // Convert "import module" to direct variable assignments
        // Handle module aliasing: "import module as alias"
    }

    fn transform_import_from(&mut self, import_from: &ImportFrom) -> Vec<Stmt> {
        // Convert "from module import name" to direct assignments
        // Handle relative imports by resolving to absolute module references
        // Manage name conflicts through renaming
    }
}
```

#### 3.2 Name Conflict Resolution

Implement scope-aware name mangling:

```rust
pub struct NameMangler {
    module_prefixes: HashMap<ModuleId, String>,
    global_names: HashSet<String>,
    conflicts: HashMap<String, Vec<ModuleId>>,
}

impl NameMangler {
    fn mangle_name(&self, name: &str, module: ModuleId) -> String {
        if self.has_conflict(name) {
            format!("{}_{}", self.module_prefixes[&module], name)
        } else {
            name.to_string()
        }
    }
}
```

#### 3.3 Module Concatenation

Combine modules with proper scoping:

```rust
impl ModuleConcatenator {
    fn concatenate_modules(&self, graph: &ModuleGraph) -> Suite {
        let mut combined_statements = Vec::new();

        for module_id in self.topological_sort(graph) {
            let module = &graph.modules[&module_id];
            let transformed = self.transform_module(module);
            combined_statements.extend(transformed);
        }

        Suite {
            body: combined_statements,
        }
    }
}
```

### Phase 4: Tree Shaking Implementation

#### 4.1 Usage Analysis

Track which names are actually used across modules:

```rust
pub struct UsageTracker {
    used_names: HashMap<ModuleId, HashSet<String>>,
    exported_names: HashMap<ModuleId, HashSet<String>>,
}

impl UsageTracker {
    fn mark_used(&mut self, name: &str, from_module: ModuleId) {
        // Mark name as used and propagate to dependencies
        self.propagate_usage(name, from_module);
    }

    fn get_unused_exports(&self, module: ModuleId) -> HashSet<String> {
        // Return exports that are never imported by other modules
    }
}
```

#### 4.2 Dead Code Elimination

Remove unused functions, classes, and variables:

```rust
impl DeadCodeEliminator {
    fn eliminate_unused(&self, ast: &mut Suite, used_names: &HashSet<String>) {
        ast.body.retain(|stmt| match stmt {
            Stmt::FunctionDef(func) => used_names.contains(&func.name),
            Stmt::ClassDef(class) => used_names.contains(&class.name),
            Stmt::Assign(assign) => self.has_used_targets(&assign.targets, used_names),
            _ => true, // Keep statements with potential side effects
        });
    }
}
```

### Phase 5: Code Generation and Output

#### 5.1 AST Unparsing

Use rustpython-unparser for reliable code generation:

```rust
pub struct CodeGenerator {
    unparser: unparser::Unparser,
}

impl CodeGenerator {
    fn generate_bundle(&self, transformed_ast: &Suite) -> Result<String> {
        // Use rustpython-unparser to convert AST back to Python code
        // Ensure proper formatting and Python syntax compliance
        self.unparser.unparse(transformed_ast)
    }

    fn add_bundle_header(&self, code: &str) -> String {
        // Add header comments explaining the bundle
        // Include any necessary runtime setup
        format!("# Generated by Serpen Python Bundler\n{}", code)
    }
}
```

#### 5.2 Source Map Generation (Future Enhancement)

Track original source locations for debugging:

```rust
pub struct SourceMapGenerator {
    mappings: Vec<SourceMapping>,
}

struct SourceMapping {
    generated_line: u32,
    generated_column: u32,
    original_file: PathBuf,
    original_line: u32,
    original_column: u32,
}
```

## Implementation Roadmap

### Step 1: Core Infrastructure (Week 1-2)

1. Add rustpython-unparser dependency to Cargo.toml
2. Implement basic ModuleGraph and ModuleNode structures
3. Create PyAstVisitor trait and basic visitor framework
4. Fix existing Int conversion errors in bundler.rs

### Step 2: Module Discovery (Week 2-3)

1. Implement ModuleResolver with Python import semantics
2. Build module graph construction from entry points
3. Add support for relative imports and package resolution
4. Create comprehensive test suite for module discovery

### Step 3: AST Transformation (Week 3-4)

1. Implement BundleTransformer with import/export rewriting
2. Add NameMangler for conflict resolution
3. Create ModuleConcatenator for combining modules
4. Test transformation with simple multi-module examples

### Step 4: Tree Shaking (Week 4-5)

1. Implement UsageTracker for cross-module analysis
2. Add DeadCodeEliminator for unused code removal
3. Handle edge cases like star imports and dynamic references
4. Validate tree shaking with complex dependency scenarios

### Step 5: Code Generation (Week 5-6)

1. Integrate rustpython-unparser for AST-to-code conversion
2. Implement CodeGenerator with proper formatting
3. Add bundle metadata and header generation
4. Create end-to-end testing for complete bundling pipeline

### Step 6: Testing and Validation (Week 6-7)

1. Test with real Python packages and dependencies
2. Validate output correctness and execution behavior
3. Performance benchmarking and optimization
4. Documentation and usage examples

## Technical Challenges and Solutions

### Challenge 1: Python Import Semantics

**Problem**: Python's complex import system (relative imports, packages, namespace packages)
**Solution**: Implement comprehensive ModuleResolver that understands Python's import algorithm

### Challenge 2: Name Conflicts During Concatenation

**Problem**: Multiple modules may define the same names
**Solution**: Implement scope-aware NameMangler with module prefixing and conflict detection

### Challenge 3: Preserving Python Semantics

**Problem**: Module concatenation must preserve Python's execution model
**Solution**: Careful analysis of module initialization order and side effect preservation

### Challenge 4: AST Node Complexity

**Problem**: Python AST has many node types and edge cases
**Solution**: Comprehensive visitor pattern implementation with systematic testing

### Challenge 5: Tree Shaking Accuracy

**Problem**: Dynamic features like `getattr()` make static analysis difficult
**Solution**: Conservative approach with user-configurable analysis depth

## Success Metrics

### 1. Stickytape Compatibility Validation

- **Pass all Stickytape test fixtures**: Successfully bundle and execute all test scenarios from Stickytape's test suite
- **Import pattern compatibility**: Handle all Python import patterns that Stickytape supports:
  - Relative imports (`from .module import func`)
  - Absolute imports (`from package.module import Class`)
  - Star imports (`from module import *`)
  - Aliased imports (`import module as alias`)
  - Nested package imports (`from package.subpackage import item`)
- **Execution equivalence**: Bundled output produces identical results to original modular code across all Stickytape fixtures

### 2. Functional Correctness

- **Semantic preservation**: Bundled code executes identically to original modular code in all scenarios
- **Error handling**: Proper error propagation and stack traces in bundled code
- **Side effect preservation**: Module initialization order and side effects maintained
- **Circular import handling**: Proper resolution of circular dependencies

### 3. Python Feature Completeness

- **Import system coverage**: Support for all common Python import patterns and edge cases
- **Package structure**: Handle namespace packages, regular packages, and single modules
- **Dynamic imports**: Basic support for `__import__()` and `importlib` patterns where statically analyzable
- **Python version compatibility**: Work across Python 3.8+ versions

### 4. Performance and Optimization

- **Bundling performance**: Bundling time scales linearly with codebase size
- **Tree shaking effectiveness**: Bundle size reduction of 20-50% on typical projects through dead code elimination
- **Memory efficiency**: Handle large codebases without excessive memory usage
- **Incremental bundling**: Support for efficient re-bundling on file changes

### 5. Developer Experience

- **Error diagnostics**: Clear, actionable error messages with source location information
- **Build integration**: Seamless integration with common Python build tools and workflows
- **Documentation**: Comprehensive usage examples and migration guides
- **Maintainability**: Clean, well-tested codebase that enables future enhancements

### 6. Validation Benchmarks

- **Stickytape test suite**: 100% pass rate on adapted Stickytape test fixtures
- **Real-world packages**: Successful bundling of popular Python packages (requests, click, etc.)
- **Edge case coverage**: Handle complex scenarios like recursive imports, conditional imports, and dynamic module loading
- **Regression testing**: Comprehensive test suite preventing functionality regressions

## Conclusion

This strategy adapts proven JavaScript bundler techniques for Python's unique characteristics. By implementing AST rewriting with proper module graph construction, scope management, and tree shaking, Serpen can achieve reliable Python bundling that preserves correctness while enabling optimization. The phased approach ensures systematic development with comprehensive testing at each stage.

The key insight from JavaScript bundler analysis is that AST-based transformation provides much more reliable results than text-based concatenation, enabling sophisticated optimizations while maintaining semantic correctness. Adapting these techniques for Python requires careful attention to Python's import system and execution model, but the fundamental patterns translate well.
