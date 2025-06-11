# Turbopack Architecture Analysis for Cribo

## Executive Summary

This document analyzes Turbopack's architecture for symbol tracking, conflict resolution, and module bundling. Turbopack, Next.js's Rust-based bundler successor to Webpack, introduces innovative concepts like the Turbo-Tasks computation framework and sophisticated module analysis. These patterns offer valuable insights for building a high-performance Python bundler.

## Architectural Overview

### Core Design Principles

1. **Incremental Computation**: Built on Turbo-Tasks for automatic memoization and invalidation
2. **Parallelism First**: Designed for maximum parallel execution
3. **Module Graph Analysis**: Deep understanding of dependencies before transformation
4. **Flexible Chunking**: Sophisticated strategies for code splitting

## Symbol Tracking Architecture

### Import Analysis System

Turbopack's import tracking is implemented through a comprehensive `ImportMap`:

```rust
pub struct ImportMap {
    /// Map from local identifier to symbol
    imports: FxHashMap<Id, Symbol>,

    /// Namespace imports: `import * as ns from "..."`
    namespace_imports: FxHashMap<Id, ImportAnnotation>,

    /// Import annotations (webpack magic comments)
    import_annotations: FxHashMap<Symbol, ImportAnnotation>,

    /// Re-exports tracking
    reexports: Vec<Reexport>,

    /// Star imports that need special handling
    star_imports: Vec<ImportAnnotation>,
}

pub enum Symbol {
    /// Direct symbol reference
    Direct(Id),
    /// Nested access like ns.foo.bar
    Nested(Box<Symbol>, Id),
    /// Symbol from another module
    External(ModuleId, Box<Symbol>),
}
```

### Dependency Graph

The `DepGraph` provides fine-grained dependency tracking:

```rust
pub struct DepGraph {
    // Item definitions
    items: FxHashMap<ItemId, Item>,

    // Dependencies between items
    deps: FxHashMap<ItemId, Vec<Dep>>,

    // Side effects ordering
    side_effects: Vec<ItemId>,

    // Variable state tracking
    var_states: FxHashMap<Id, VarState>,
}

pub struct VarState {
    declarator: Option<ItemId>,
    last_writes: Vec<ItemId>,
    last_reads: Vec<ItemId>,
    last_op: Option<VarOp>,
}

pub enum Dep {
    // Strong dependency (always needed)
    Strong(ItemId),
    // Weak dependency (only if target is included)
    Weak(ItemId),
}
```

### Module Reference System

Abstract trait hierarchy for tracking references:

```rust
#[turbo_tasks::value_trait]
pub trait ModuleReference {
    /// The module being referenced
    fn resolve_reference(&self) -> Vc<ResolveResult>;

    /// How this reference affects chunking
    fn chunking_behavior(&self) -> ChunkingType;
}

pub enum ChunkingType {
    /// Load in parallel with parent
    Parallel,
    /// Load asynchronously
    Async,
    /// Load in isolated context
    Isolated,
    /// Can be shared between chunks
    Shared,
}
```

## AST Processing Architecture

### Visitor Infrastructure

Turbopack leverages SWC's visitor pattern with custom enhancements:

```rust
pub trait AstModifier {
    fn modifier(&self) -> Vc<Box<dyn VisitMut>>;
}

// Path-aware visiting
pub struct AstPath<'a> {
    path: Vec<AstParentKind>,
    visitor: &'a mut dyn VisitMut,
}

pub enum AstParentKind {
    FunctionDecl,
    FunctionExpr,
    ArrowExpr,
    BlockStmt,
    // ... more node types
}
```

### Symbol Analysis

Multi-phase analysis approach:

```rust
pub struct Analyzer<'a> {
    // Current scope stack
    scopes: Vec<Scope>,

    // Variable tracking
    vars: FxHashMap<Id, VarState>,

    // Import/export analysis
    imports: ImportMap,
    exports: ExportMap,

    // Side effect tracking
    last_side_effects: Vec<ItemId>,

    // Module evaluation state
    eval_state: EvalState,
}

impl Analyzer<'_> {
    fn analyze_module(&mut self, module: &Module) -> AnalysisResult {
        // Phase 1: Collect all declarations
        self.collect_declarations(module);

        // Phase 2: Build dependency graph
        self.build_dependencies(module);

        // Phase 3: Analyze side effects
        self.analyze_side_effects(module);

        // Phase 4: Determine tree shaking boundaries
        self.compute_shake_points(module);
    }
}
```

## Module System Architecture

### Module Abstraction Hierarchy

```rust
#[turbo_tasks::value_trait]
pub trait Module {
    /// Module identifier
    fn identifier(&self) -> Vc<ModuleId>;

    /// Module content
    fn content(&self) -> Vc<ModuleContent>;

    /// References to other modules
    fn references(&self) -> Vc<ModuleReferences>;
}

#[turbo_tasks::value_trait]
pub trait ChunkableModule: Module {
    /// Convert to chunk item
    fn as_chunk_item(&self, context: Vc<ChunkingContext>) -> Vc<Box<dyn ChunkItem>>;
}

#[turbo_tasks::value_trait]
pub trait ChunkItem {
    /// Generate code for this item
    fn code(&self) -> Vc<Code>;

    /// Item identifier within chunk
    fn id(&self) -> Vc<ModuleId>;
}
```

### Module ID Strategies

Flexible module identification system:

```rust
pub enum ModuleIdStrategy {
    /// Use full paths (development)
    Dev,
    /// Use optimized IDs (production)
    Optimized {
        map: HashMap<String, u32>,
        counter: AtomicU32,
    },
}

impl ModuleIdStrategy {
    fn module_id(&self, path: &str) -> ModuleId {
        match self {
            Self::Dev => ModuleId::String(path.to_string()),
            Self::Optimized { map, counter } => {
                if let Some(&id) = map.get(path) {
                    ModuleId::Number(id)
                } else {
                    // Generate new ID
                    let id = counter.fetch_add(1, Ordering::SeqCst);
                    // Ensure within JS safe integer range
                    if id < MAX_SAFE_INTEGER {
                        ModuleId::Number(id)
                    } else {
                        // Fallback to hash
                        ModuleId::String(hash_path(path))
                    }
                }
            }
        }
    }
}
```

## Turbo-Tasks Framework

### Core Concepts

Turbo-Tasks provides automatic memoization and invalidation:

```rust
#[turbo_tasks::function]
async fn analyze_module(module: Vc<Box<dyn Module>>) -> Result<Vc<AnalysisResult>> {
    // This function is automatically memoized
    // Re-runs only when inputs change
    let content = module.content().await?;
    let ast = parse_content(content).await?;
    let analysis = analyze_ast(ast).await?;
    Ok(analysis)
}

// Value types are automatically tracked
#[turbo_tasks::value]
pub struct AnalysisResult {
    imports: ImportMap,
    exports: ExportMap,
    dep_graph: DepGraph,
}
```

### Invalidation System

```rust
// Automatic dependency tracking
#[turbo_tasks::function]
async fn bundle_module(
    module: Vc<Box<dyn Module>>,
    context: Vc<BundleContext>,
) -> Result<Vc<BundledModule>> {
    // If module changes, this re-runs
    let analysis = analyze_module(module).await?;

    // If context changes, this re-runs
    let options = context.options().await?;

    // Compute bundle
    let bundled = compute_bundle(analysis, options).await?;
    Ok(bundled)
}
```

## Conflict Resolution Strategies

### Tree Shaking with Module Parts

Turbopack can split modules into parts for fine-grained tree shaking:

```rust
pub struct ModulePart {
    /// Original module
    module: Vc<Box<dyn Module>>,
    /// Part identifier
    part: PartId,
    /// Exported symbols from this part
    exports: Vec<Symbol>,
    /// Internal dependencies
    deps: Vec<PartId>,
}

// Generate parts based on usage
fn split_module(module: Vc<Box<dyn Module>>, used_exports: &HashSet<Symbol>) -> Vec<ModulePart> {
    let analysis = analyze_module(module);
    let dep_graph = &analysis.dep_graph;

    // Find minimal set of items needed for used exports
    let required_items = compute_required_items(dep_graph, used_exports);

    // Group items into parts based on dependencies
    let parts = group_into_parts(required_items, dep_graph);

    parts
}
```

### Facade Module Generation

For re-exports and public APIs:

```rust
pub struct FacadeModule {
    /// Internal module being wrapped
    inner: Vc<Box<dyn Module>>,
    /// Exposed exports
    exports: Vec<Export>,
}

impl Module for FacadeModule {
    fn code(&self) -> Vc<Code> {
        let inner_id = self.inner.id();
        let mut code = String::new();

        // Generate re-export code
        for export in &self.exports {
            match export {
                Export::Named(name) => {
                    write!(code, "export {{ {} }} from {:?};\n", name, inner_id);
                }
                Export::Default => {
                    write!(code, "export {{ default }} from {:?};\n", inner_id);
                }
                Export::All => {
                    write!(code, "export * from {:?};\n", inner_id);
                }
            }
        }

        Code::from(code)
    }
}
```

## Reusable Crates Analysis

### 1. `turbo-tasks` - Incremental Computation Framework

**Key Features**:

- Automatic memoization of async functions
- Fine-grained invalidation tracking
- Parallel execution with dependency management
- Could be adapted for Python AST analysis

**Potential Uses**:

```rust
// Python module analysis with caching
#[turbo_tasks::function]
async fn analyze_python_module(path: Vc<String>) -> Result<Vc<PythonAnalysis>> {
    let content = read_file(path).await?;
    let ast = parse_python(content).await?;
    let analysis = analyze_ast(ast).await?;
    Ok(analysis)
}
```

### 2. `turbo-tasks-fs` - Virtual File System

**Features**:

- Async file operations
- File watching with invalidation
- Virtual file system support
- Glob pattern matching

**Benefits for Python**:

- Handle Python's complex module search paths
- Support for namespace packages
- Virtual modules for bundled output

### 3. `turbopack-core` Abstractions

**Reusable Components**:

```rust
// Module resolution
pub trait ResolveModule {
    fn resolve(&self, request: &str) -> Vc<ResolveResult>;
}

// Asset abstraction
pub trait Asset {
    fn content(&self) -> Vc<AssetContent>;
    fn identifier(&self) -> Vc<String>;
}

// Chunk optimization
pub trait ChunkOptimizer {
    fn optimize(&self, chunks: Vec<Chunk>) -> Vec<Chunk>;
}
```

### 4. `turbopack-css` - CSS Processing Patterns

While CSS-specific, the architecture patterns for:

- Import resolution in non-JS languages
- Source map generation
- Module composition

Could be adapted for Python's import system.

### 5. Generic Utilities

**Data Structures**:

- `FxIndexMap`/`FxIndexSet`: Order-preserving collections
- `RcStr`: Reference-counted strings (similar to Rspack's interned strings)
- Visitor utilities for AST traversal

**Algorithms**:

- Topological sorting for module ordering
- Cycle detection in dependency graphs
- Chunking optimization algorithms

## Architectural Patterns for Python Adaptation

### 1. Multi-Phase Analysis

```rust
pub struct PythonAnalyzer {
    // Phase 1: Symbol collection
    symbols: SymbolTable,

    // Phase 2: Import resolution
    imports: ImportResolver,

    // Phase 3: Dependency graph
    dep_graph: DepGraph,

    // Phase 4: Tree shaking
    shake_analyzer: TreeShaker,
}

impl PythonAnalyzer {
    async fn analyze(&mut self, module: &PythonModule) -> Result<Analysis> {
        // Each phase can run incrementally
        let symbols = self.collect_symbols(module).await?;
        let imports = self.resolve_imports(module, &symbols).await?;
        let deps = self.build_dep_graph(&symbols, &imports).await?;
        let shake_points = self.analyze_tree_shaking(&deps).await?;

        Ok(Analysis {
            symbols,
            imports,
            deps,
            shake_points,
        })
    }
}
```

### 2. Python-Specific Module Types

```rust
pub enum PythonModuleType {
    /// Regular .py file
    Regular(RegularModule),
    /// Package __init__.py
    Package(PackageModule),
    /// Namespace package (no __init__.py)
    Namespace(NamespaceModule),
    /// Extension module (.so/.pyd)
    Extension(ExtensionModule),
    /// Stub file (.pyi)
    Stub(StubModule),
}
```

### 3. Import Context Handling

```rust
pub struct PythonImportContext {
    /// Current module path
    current_module: PathBuf,
    /// Python path entries
    python_path: Vec<PathBuf>,
    /// Import level for relative imports
    import_level: u32,
    /// Whether inside TYPE_CHECKING block
    type_checking: bool,
}
```

## Implementation Recommendations

### Immediate Adoptions

1. **Module Abstraction Hierarchy**: Use trait-based module system
2. **Reference Tracking**: Implement `ModuleReference` pattern
3. **Two-Phase Processing**: Separate analysis from transformation

### Medium-Term Goals

1. **Turbo-Tasks Integration**: For incremental computation
2. **Virtual File System**: For better testing and module resolution
3. **Chunk Optimization**: Adapt algorithms for Python bundles

### Long-Term Vision

1. **Full Incremental Pipeline**: Every step memoized and invalidated
2. **Parallel Analysis**: Leverage Turbo-Tasks for parallel processing
3. **Advanced Tree Shaking**: Module splitting for partial imports

## Key Insights

### Strengths of Turbopack's Approach

1. **Incremental by Design**: Not bolted on afterward
2. **Parallelism First**: Automatic parallel execution
3. **Deep Analysis**: Understands code at a fine-grained level
4. **Flexible Architecture**: Clean abstractions allow extensions

### Python-Specific Considerations

1. **Dynamic Imports**: Need runtime fallbacks
2. **Module Attributes**: `__all__`, `__name__`, `__file__`
3. **Import Hooks**: Support for custom importers
4. **Type Checking**: Handle TYPE_CHECKING blocks specially

### Performance Opportunities

1. **Memoization**: Cache AST parsing and analysis
2. **Incremental Updates**: Only reanalyze changed modules
3. **Parallel Processing**: Analyze independent modules concurrently
4. **Lazy Evaluation**: Only process what's needed for output

## Conclusion

Turbopack represents the cutting edge of bundler architecture with its Turbo-Tasks framework and sophisticated module analysis. While more complex than Rolldown or Rspack, it offers unique benefits:

1. **Automatic Incrementality**: Changes propagate minimally
2. **Parallel by Default**: No manual parallelization needed
3. **Deep Code Understanding**: Can optimize more aggressively

For Cribo, adopting Turbopack's patterns—particularly the module abstraction hierarchy and reference tracking—while selectively using Turbo-Tasks for performance-critical paths could provide an excellent balance of capability and complexity.

The key is to start with the simpler patterns (module traits, reference tracking) and gradually adopt more sophisticated features (Turbo-Tasks, incremental computation) as the bundler matures.
