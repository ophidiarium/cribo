# Rspack Architecture Analysis for Cribo

## Executive Summary

This document analyzes Rspack's architecture for symbol tracking, conflict resolution, and module bundling, identifying patterns and components that could benefit Cribo. Rspack, a Rust-based webpack-compatible bundler, provides sophisticated symbol management and module concatenation capabilities that offer valuable insights for Python bundling.

## Architectural Overview

### Core Design Principles

1. **Webpack Compatibility**: Maintains API compatibility while reimplementing core logic in Rust
2. **Performance Focus**: Uses interned strings, efficient data structures, and parallel processing
3. **Incremental Compilation**: Designed for fast rebuilds with granular dependency tracking
4. **Plugin Architecture**: Extensible through hooks and plugin system

## Symbol Tracking Architecture

### Identifier System

Rspack uses an efficient identifier system based on interned strings:

```rust
// Interned string wrapper for zero-cost comparison
pub struct Identifier(Ustr);

impl Identifier {
    pub fn from(s: impl AsRef<str>) -> Self {
        Self(s.as_ref().into())
    }
}

// Custom hasher optimized for interned strings
pub struct IdentifierHasher {
    hash: u64,
}
```

**Benefits**:

- O(1) string comparison
- Reduced memory usage (single string instance)
- Cache-friendly hash computation

### Module Graph Architecture

The central data structure tracking all module relationships:

```rust
pub struct ModuleGraph {
    // Primary storage
    modules: IdentifierMap<Option<BoxModule>>,
    dependencies: HashMap<DependencyId, Option<BoxDependency>>,

    // Graph relationships
    module_graph_modules: IdentifierMap<Option<ModuleGraphModule>>,
    connections: HashMap<DependencyId, Option<ModuleGraphConnection>>,

    // Export tracking
    exports_info_map: UkeyMap<ExportsInfo, ExportsInfoData>,
}

pub struct ModuleGraphModule {
    // Incoming connections
    incoming_connections: HashSet<ConnectionId>,
    // Outgoing connections
    outgoing_connections: HashSet<ConnectionId>,
    // Module metadata
    exports_type: Option<ExportsType>,
    has_side_effects: Option<bool>,
}
```

### Connection Tracking

Sophisticated dependency tracking system:

```rust
pub struct ModuleGraphConnection {
    dependency_id: DependencyId,
    origin_module_identifier: Option<ModuleIdentifier>,
    resolved_module_identifier: Option<ModuleIdentifier>,
    resolved_original_module_identifier: Option<ModuleIdentifier>,

    // Connection metadata
    conditional: bool,
    weak: bool,

    // For optimizations
    user_request: Option<String>,
    resolved_request: Option<String>,
}
```

## AST Processing and Scope Management

### Parser Architecture

Rspack uses SWC (Speedy Web Compiler) with custom visitor implementation:

```rust
pub struct JavascriptParser {
    // Scope tracking
    definitions_db: DefinitionsDatabase,
    scopes: Vec<ScopeInfo>,

    // Current parsing context
    module_identifier: ModuleIdentifier,
    build_info: BuildInfo,
    build_meta: BuildMeta,

    // AST walker
    walker: AstWalker,
}

// Scope management methods
impl JavascriptParser {
    fn in_block_scope<F>(&mut self, f: F) -> Result<()> {
        self.scopes.push(ScopeInfo::new(ScopeType::Block));
        let result = f(self);
        self.scopes.pop();
        result
    }

    fn in_function_scope<F>(&mut self, f: F) -> Result<()> {
        self.scopes.push(ScopeInfo::new(ScopeType::Function));
        let result = f(self);
        self.scopes.pop();
        result
    }
}
```

### Definition Tracking

Track variable definitions across scopes:

```rust
pub struct DefinitionsDatabase {
    // Variable definitions by scope
    definitions: HashMap<ScopeId, HashMap<String, DefinitionInfo>>,
}

pub struct DefinitionInfo {
    name: String,
    definition_type: DefinitionType,
    can_rename: bool,
    used_by_exports: bool,
}

pub enum DefinitionType {
    Function,
    Variable,
    Class,
    Import,
    Parameter,
}
```

## Conflict Resolution Strategies

### Module Concatenation

Rspack implements sophisticated module concatenation with symbol renaming:

```rust
pub struct ConcatenatedModule {
    modules: Vec<Module>,
    // Symbol renaming map
    symbol_bindings: HashMap<String, SymbolBinding>,
}

pub enum SymbolBinding {
    // Direct reference to original symbol
    Raw(String),
    // Renamed symbol reference
    Symbol(Identifier),
}

// Module reference generation for concatenated modules
fn generate_module_reference(index: u32, export_data: &str) -> String {
    format!(
        "__WEBPACK_MODULE_REFERENCE__{}_{}{}{}{}__",
        index, export_data, call_flag, direct_import_flag, asi_safe_flag
    )
}
```

### Name Mangling

Deterministic name generation for minification:

```rust
pub struct MangleExportsPlugin {
    deterministic: bool,
    reserved: HashSet<&'static str>,
}

// Reserved JavaScript names (188 total)
pub const RESERVED_NAMES: [&str; 188] = [
    "__WEBPACK_DEFAULT_EXPORT__",
    "__WEBPACK_NAMESPACE_OBJECT__",
    "abstract",
    "arguments",
    "async",
    "await",
    "boolean",
    "break",
    "byte",
    "case",
    // ... more reserved words
];

// Generate short identifiers deterministically
fn number_to_identifier(n: u32) -> String {
    const CHARS_START: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$";
    const CHARS_CONT: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_$";

    if n < CHARS_START.len() as u32 {
        return char::from(CHARS_START[n as usize]).to_string();
    }

    // Generate aa, ab, ac, ... style names
    let mut result = String::new();
    let mut num = n - CHARS_START.len() as u32;

    result.push(char::from(CHARS_START[(num % CHARS_START.len()) as usize]));
    num /= CHARS_START.len() as u32;

    while num > 0 {
        num -= 1;
        result.push(char::from(CHARS_CONT[(num % CHARS_CONT.len()) as usize]));
        num /= CHARS_CONT.len() as u32;
    }

    result
}
```

## Circular Dependency Handling

### Detection Algorithm

```rust
pub struct CircularDependencyPlugin {
    handle_error: CircularDependencyHandleError,
    exclude_patterns: Vec<Regex>,
}

pub enum CircularDependencyHandleError {
    Error,
    Warning,
}

// DFS-based cycle detection
fn detect_circular_dependencies(
    module_graph: &ModuleGraph,
    entry: ModuleIdentifier,
) -> Vec<Vec<ModuleIdentifier>> {
    let mut visited = HashSet::new();
    let mut stack = Vec::new();
    let mut cycles = Vec::new();

    fn dfs(
        module: ModuleIdentifier,
        graph: &ModuleGraph,
        visited: &mut HashSet<ModuleIdentifier>,
        stack: &mut Vec<ModuleIdentifier>,
        cycles: &mut Vec<Vec<ModuleIdentifier>>,
    ) {
        if stack.contains(&module) {
            // Found cycle
            let start = stack.iter().position(|&m| m == module).unwrap();
            cycles.push(stack[start..].to_vec());
            return;
        }

        if visited.contains(&module) {
            return;
        }

        visited.insert(module);
        stack.push(module);

        // Get aggregated dependencies (not async imports)
        for dep in get_aggregated_dependencies(graph, module) {
            if let Some(target) = dep.resolved_module {
                dfs(target, graph, visited, stack, cycles);
            }
        }

        stack.pop();
    }

    dfs(entry, module_graph, &mut visited, &mut stack, &mut cycles);
    cycles
}
```

## Reusable Crates Analysis

### 1. `rspack_util` - General Utilities

**Key Components**:

- `Identifier`: Interned string system
- `to_identifier()`: Convert any string to valid JS identifier
- Hash utilities for content hashing
- Path manipulation utilities

**Adaptability for Python**:

- Identifier system works for any language
- Path utilities are generic
- Hash functions are language-agnostic

### 2. `rspack_collections` - Optimized Collections

```rust
// Specialized maps for identifiers
pub type IdentifierMap<V> = HashMap<Identifier, V, BuildHasherDefault<IdentifierHasher>>;
pub type IdentifierIndexMap<V> = IndexMap<Identifier, V, BuildHasherDefault<IdentifierHasher>>;
pub type IdentifierSet = HashSet<Identifier, BuildHasherDefault<IdentifierHasher>>;

// Unique key map for efficient storage
pub struct UkeyMap<K, V> {
    inner: Vec<Option<V>>,
    _marker: PhantomData<K>,
}
```

### 3. `rspack_error` - Error Handling

Comprehensive error system with:

- Diagnostic information (file, line, column)
- Error severity levels
- Structured error types
- Rich formatting support

```rust
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Option<String>,
    pub title: String,
    pub file_path: Option<PathBuf>,
    pub source_code: Option<String>,
    pub start: Option<Position>,
    pub end: Option<Position>,
}
```

### 4. `rspack_hash` - Content Hashing

Multiple hash implementations:

- xxhash64 for fast non-cryptographic hashing
- SHA implementations for content integrity
- Configurable hash functions

## Architectural Patterns for Python Adaptation

### 1. Symbol Tracking Adaptations

**Python-Specific Considerations**:

- Track Python scopes: module, class, function, comprehension
- Handle Python's LEGB (Local, Enclosing, Global, Built-in) resolution
- Support for `__all__` exports
- Dynamic import tracking (`importlib`, `__import__`)

**Proposed Structure**:

```rust
pub struct PythonSymbolTracker {
    // Module-level symbols
    module_symbols: IdentifierMap<SymbolInfo>,
    // Class and function scopes
    scopes: Vec<PythonScope>,
    // Built-in tracking
    builtins: HashSet<Identifier>,
}

pub enum PythonScope {
    Module,
    Class { name: Identifier },
    Function { name: Identifier, is_async: bool },
    Comprehension { type: ComprehensionType },
}
```

### 2. Import Resolution

**Python Import Types**:

```rust
pub enum PythonImport {
    // import module
    Import {
        module: String,
    },
    // from module import name
    FromImport {
        module: String,
        names: Vec<ImportName>,
    },
    // from . import module
    RelativeImport {
        level: u32,
        module: Option<String>,
        names: Vec<ImportName>,
    },
    // import module as alias
    ImportAs {
        module: String,
        alias: String,
    },
}

pub struct ImportName {
    name: String,
    alias: Option<String>,
}
```

### 3. Conflict Resolution Strategy

**Python Reserved Names**:

```rust
pub const PYTHON_BUILTINS: &[&str] = &[
    // Built-in functions
    "abs",
    "all",
    "any",
    "ascii",
    "bin",
    "bool",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    // ... more builtins

    // Keywords
    "False",
    "None",
    "True",
    "and",
    "as",
    "assert",
    "async",
    "await",
    "break",
    "class",
    "continue",
    "def",
    "del",
    // ... more keywords
];
```

## Implementation Recommendations

### Phase 1: Core Infrastructure

1. **Adopt Identifier System**
   - Implement interned strings for Python identifiers
   - Use specialized hasher for performance

2. **Module Graph Structure**
   - Port module graph architecture
   - Adapt for Python's import system

3. **Error Handling**
   - Use `rspack_error` patterns for diagnostics
   - Python-specific error types

### Phase 2: Symbol Management

1. **Scope Tracking**
   - Implement Python scope hierarchy
   - Track variable bindings per scope

2. **Import Resolution**
   - Handle all Python import forms
   - Track re-exports through `__all__`

3. **Conflict Detection**
   - Build comprehensive symbol table
   - Detect naming conflicts early

### Phase 3: Optimization

1. **Name Mangling**
   - Adapt deterministic naming algorithm
   - Respect Python naming conventions

2. **Module Concatenation**
   - Implement Python module inlining
   - Handle namespace preservation

3. **Circular Dependencies**
   - Adapt DFS algorithm for Python
   - Consider Python's lazy import semantics

## Key Takeaways

### Strengths to Adopt

1. **Interned String System**: Significant performance boost for identifier handling
2. **Module Graph Architecture**: Proven pattern for complex dependency tracking
3. **Visitor Pattern with Context**: Clean way to handle AST traversal with scope
4. **Deterministic Output**: Important for reproducible builds
5. **Comprehensive Error Handling**: Professional-grade diagnostics

### Python-Specific Adaptations Needed

1. **Different Scope Rules**: LEGB vs JavaScript's lexical scoping
2. **Import Complexity**: Relative imports, namespace packages, `__init__.py`
3. **Dynamic Features**: `getattr`, `__import__`, `importlib` usage
4. **Module Attributes**: Python modules are objects with attributes
5. **Type Annotations**: Need special handling for type hints

## Conclusion

Rspack provides a mature, performance-oriented architecture for bundling with sophisticated symbol tracking and conflict resolution. While designed for JavaScript, many of its core patterns—particularly the module graph, identifier system, and scope tracking—can be adapted for Python bundling.

The key insight is that Rspack separates language-specific concerns (JavaScript parsing) from generic bundling concerns (module graphs, symbol tracking, conflict resolution). This separation makes it easier to adapt their patterns for Python while respecting Python's unique semantics.

By combining Rspack's architectural patterns with Python-specific adaptations, Cribo can achieve professional-grade bundling capabilities with excellent performance characteristics.
