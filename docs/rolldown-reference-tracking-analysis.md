# Rolldown Reference Tracking Architecture Analysis

This document analyzes Rolldown's architecture for reference tracking, symbol resolution, and module graph management to identify patterns applicable to our Python bundler.

## Key Architectural Components

### 1. Symbol Reference Database (`SymbolRefDb`)

Rolldown uses a two-tier symbol reference system:

```rust
// Global symbol database
pub struct SymbolRefDb {
    inner: IndexVec<ModuleIdx, Option<SymbolRefDbForModule>>,
}

// Per-module symbol information
pub struct SymbolRefDbForModule {
    owner_idx: ModuleIdx,
    root_scope_id: ScopeId,
    pub ast_scopes: AstScopes,
    pub flags: FxHashMap<SymbolId, SymbolRefFlags>,
    pub classic_data: IndexVec<SymbolId, SymbolRefDataClassic>,
}
```

**Key Features:**

- **Facade Symbols**: Virtual symbols created for imports/exports that don't exist in AST
- **Symbol Linking**: Union-find data structure for efficient symbol resolution
- **Namespace Aliases**: Special handling for CommonJS interop (e.g., `import {a} from 'cjs'` → `cjs_ns.a`)
- **Symbol Flags**: Track const declarations, reassignment status

### 2. AST Scanner Pattern

The AST scanner (`AstScanner`) performs a single-pass analysis collecting:

```rust
pub struct ScanResult {
    pub named_imports: FxIndexMap<SymbolRef, NamedImport>,
    pub named_exports: FxHashMap<Rstr, LocalExport>,
    pub stmt_infos: StmtInfos,
    pub import_records: IndexVec<ImportRecordIdx, RawImportRecord>,
    pub symbol_ref_db: SymbolRefDbForModule,
    pub resolved_member_expr_refs: MemberExprRefResolutionMap,
    // ... more fields
}
```

**Python Adaptation Ideas:**

- Track Python imports: `import foo`, `from foo import bar`, `from foo import *`
- Handle Python-specific patterns: `__all__`, dynamic imports via `importlib`
- Track attribute access chains for namespace resolution

### 3. Import/Export Resolution

Rolldown's binding phase matches imports to exports:

```rust
enum MatchImportKind {
    Normal(MatchImportKindNormal),
    Namespace { namespace_ref: SymbolRef },
    NormalAndNamespace { namespace_ref: SymbolRef, alias: Rstr },
    Cycle,
    Ambiguous { ... },
    NoMatch,
}
```

**Resolution Algorithm:**

1. Follow import chains through re-exports
2. Handle ambiguous exports from `export *`
3. Track dependencies for side-effect modules
4. Create facade symbols for missing exports (with `shim_missing_exports` option)

### 4. Member Expression Resolution

Rolldown resolves chained property access:

```javascript
// index.js
import * as foo_ns from './foo';
foo_ns.bar_ns.c;

// foo.js
export * as bar_ns from './bar';

// bar.js
export const c = 1;
```

**Implementation:**

- Iteratively resolve each property in the chain
- Track dependencies along the resolution path
- Handle dynamic exports fallback

### 5. Circular Dependency Detection

Uses a DFS-based algorithm with execution stack tracking:

```rust
enum Status {
    ToBeExecuted(ModuleIdx),
    WaitForExit(ModuleIdx),
}
```

**Algorithm:**

1. Track execution stack during module sorting
2. Detect cycles when encountering an already-executing module
3. Collect all circular dependency chains
4. Issue warnings but continue bundling

### 6. Module Wrapping Strategy

Rolldown wraps modules for several reasons:

```rust
pub struct LinkingMetadata {
    pub wrapper_ref: Option<SymbolRef>,
    pub wrapper_stmt_info: Option<StmtInfoIdx>,
    pub wrap_kind: WrapKind,
    // ...
}
```

**Wrap Kinds:**

- CommonJS modules (wrapped with `__commonJS`)
- Modules with circular dependencies
- Modules needing isolation

## Python-Specific Adaptations

### 1. Import System Mapping

| JavaScript/TypeScript       | Python Equivalent                        |
| --------------------------- | ---------------------------------------- |
| `import foo`                | `import foo`                             |
| `import {a} from 'foo'`     | `from foo import a`                      |
| `import * as ns from 'foo'` | `import foo as ns`                       |
| `export default`            | N/A (use `__all__` or module attributes) |
| `export {a}`                | Define `a` at module level               |
| `export * from 'foo'`       | `from foo import *` (avoid in bundler)   |

### 2. Symbol Reference Structure for Python

```rust
pub struct PythonSymbolRef {
    module_idx: ModuleIdx,
    symbol_id: SymbolId,
    // Python-specific fields
    is_dunder: bool, // __name__, __file__, etc.
    from_all: bool,  // Exported via __all__
}

pub struct PythonImportRecord {
    module_request: String,
    kind: PythonImportKind,
    // Track import context
    is_conditional: bool, // Inside if/try blocks
    is_lazy: bool,        // Inside function/class
}

enum PythonImportKind {
    SimpleImport,   // import foo
    FromImport,     // from foo import bar
    FromImportAll,  // from foo import *
    RelativeImport, // from . import foo
}
```

### 3. Python-Specific Resolution Challenges

1. **Dynamic `__all__`**: Unlike static exports, Python's `__all__` can be modified at runtime
2. **Conditional Imports**: Handle imports inside `if TYPE_CHECKING:` blocks
3. **Lazy Imports**: Track imports inside functions/classes
4. **Module Attributes**: Any module-level assignment is potentially an export

### 4. Proposed Architecture for Python Bundler

```rust
// Core components adaptation
pub struct PythonBundler {
    // Module graph
    module_table: ModuleTable,
    symbol_db: SymbolRefDb,

    // Python-specific
    import_resolver: PythonImportResolver,
    namespace_tracker: NamespaceTracker,

    // Circular dependency handling
    cycle_detector: CycleDetector,
    wrapper_generator: ModuleWrapper,
}

// AST scanning for Python
pub struct PythonAstScanner {
    // Track all module-level bindings
    module_bindings: HashMap<String, SymbolRef>,

    // Track __all__ if present
    explicit_exports: Option<Vec<String>>,

    // Import tracking
    imports: Vec<PythonImportRecord>,

    // Attribute access chains
    member_accesses: Vec<MemberExprRef>,
}
```

## Implementation Recommendations

### 1. Symbol Resolution Pipeline

1. **First Pass - AST Scanning**:
   - Collect all imports and module-level bindings
   - Track `__all__` declarations
   - Record attribute access chains

2. **Second Pass - Symbol Linking**:
   - Resolve imports to their declarations
   - Build symbol reference graph
   - Detect and handle circular imports

3. **Third Pass - Tree Shaking**:
   - Mark used symbols starting from entry points
   - Propagate usage through import chains
   - Handle member expression resolution

### 2. Circular Dependency Strategy

- **Detection**: Use Rolldown's execution stack approach
- **Resolution**: Wrap circular modules in initialization functions
- **Runtime**: Generate code that handles initialization order

### 3. Module Wrapping for Python

```python
# Original module with circular dependency
# a.py
from b import func_b
def func_a(): return func_b()

# Wrapped output
def __init_a():
    global func_a
    from b import func_b
    def func_a(): return func_b()
    return locals()

# Lazy initialization on first access
_a_initialized = False
_a_exports = {}

def _get_a_export(name):
    global _a_initialized, _a_exports
    if not _a_initialized:
        _a_exports = __init_a()
        _a_initialized = True
    return _a_exports[name]
```

### 4. Member Expression Resolution

For Python attribute chains like `pkg.submodule.function`:

1. Start with the root symbol (`pkg`)
2. For each attribute access:
   - Check if it's an explicit export
   - Check if it's a submodule
   - Track all symbols that need to be included
3. Generate appropriate property access code

## Benefits of Adopting Rolldown's Architecture

1. **Proven Design**: Rolldown handles complex JavaScript module graphs efficiently
2. **Performance**: Index-based symbol references and efficient data structures
3. **Correctness**: Comprehensive handling of edge cases (cycles, ambiguous exports)
4. **Extensibility**: Clear separation between scanning, linking, and code generation phases

## Next Steps

1. Implement basic `SymbolRefDb` structure for Python
2. Create Python-specific AST scanner based on RustPython AST
3. Adapt import/export resolution for Python semantics
4. Implement circular dependency detection and handling
5. Add member expression resolution for namespace imports
