# Comprehensive AST Rewrite Implementation Plan

## Overview

This document outlines the implementation plan to make the `comprehensive_ast_rewrite` fixture pass completely. The fixture is designed as an extreme stress test for Python bundlers, featuring deeply nested naming conflicts, complex relative imports, and intentionally adversarial code patterns.

## Architectural Insights from Rolldown

Based on analysis of Rolldown's TypeScript/JavaScript bundler implementation, we can adapt several architectural patterns:

### Symbol Reference Database

Rolldown uses a two-tier symbol reference system:

- Global `SymbolRefDb` containing per-module symbol databases
- Index-based references (u32) for efficient memory usage
- Union-find data structure for symbol linking
- Facade symbols for handling virtual imports/exports

### AST Scanner Pattern

Single-pass AST scanning that collects:

- Named imports/exports with precise span information
- Statement-level metadata for tree shaking
- Symbol reference relationships
- Member expression chains for namespace resolution

### Import Resolution Algorithm

Rolldown's matching algorithm provides a robust pattern:

- Follows import chains through re-exports
- Handles ambiguous exports gracefully
- Creates placeholder symbols for missing exports
- Tracks side-effect dependencies

## Current State Analysis

### What's Working

- Basic AST rewriting for simple naming conflicts
- Module inlining with sys.modules-based approach
- Simple import resolution and aliasing
- Self-reference assignments (`process = process`)
- Basic relative import resolution

### What's Failing

The bundled code fails with undefined references due to:

1. Missing import transformations inside wrapper module functions
2. Type annotations not being renamed
3. Complex alias chains not being fully resolved
4. Cross-module reference tracking gaps

## Implementation Tasks

### Phase 1: Import Transformation in Module Wrappers

**Problem**: When modules are wrapped in init functions, their imports are not transformed to use the bundled names.

**Solution**:

1. Modify `code_generator.rs` to traverse imports inside module wrapper functions
2. Apply the same import resolution logic to imports within `__cribo_init_*` functions
3. Transform import statements to direct assignments from sys.modules

**Code Changes**:

```rust
// In code_generator.rs
fn transform_module_wrapper_imports(&mut self, module_ast: &mut Module) {
    // Visit all import statements within the module
    // Transform them to use sys.modules references
    // Example: from .user import Logger as UserLogger
    // Becomes: UserLogger = sys.modules['models.user'].Logger
}
```

### Phase 2: Type Annotation Renaming

**Problem**: Type annotations in function signatures and class definitions are not renamed when symbols are transformed.

**Solution**:

1. Implement a type annotation visitor in the AST transformer
2. Track all type annotation contexts (function args, returns, class annotations)
3. Apply symbol renames to type annotations

**Implementation Strategy**:

```rust
// New visitor for type annotations
struct TypeAnnotationRenamer<'a> {
    renames: &'a HashMap<String, String>,
}

impl<'a> Transformer for TypeAnnotationRenamer<'a> {
    fn fold_expr(&mut self, expr: Expr) -> Expr {
        match expr {
            Expr::Name(name) if self.renames.contains_key(&name.id) => Expr::Name(ExprName {
                id: self.renames[&name.id].clone(),
                ..name
            }),
            _ => walk_expr(self, expr),
        }
    }

    fn fold_annotation(&mut self, ann: Expr) -> Expr {
        self.fold_expr(ann)
    }
}
```

### Phase 3: Enhanced Alias Resolution

**Problem**: Complex alias chains like `from ..utils.helpers import validate as helper_validate` are not fully resolved. Particularly challenging are chained external references where an alias in one module is imported and aliased again in another module.

**Solution**:

1. Build a comprehensive alias resolution map during module analysis
2. Track the full chain: original name → module path → renamed symbol → alias
3. Generate proper binding statements in the bundled output
4. Handle transitive alias chains (e.g., Module A exports X as Y, Module B imports Y as Z)

**Data Structure** (adapted from Rolldown's approach):

```rust
struct AliasChain {
    original_name: String,  // "validate"
    source_module: String,  // "core.utils.helpers"
    renamed_symbol: String, // "validate_5"
    alias_name: String,     // "helper_validate"
    import_context: String, // Module where import occurs
    chain_depth: u32,       // Track re-export depth
}

// Track alias chains across modules
struct AliasResolver {
    // (module, alias) -> AliasChain
    chains: HashMap<(String, String), AliasChain>,
    // Track re-export relationships
    re_exports: HashMap<String, Vec<String>>,
}
```

**Chained Reference Resolution**:

1. When resolving an import, check if the imported symbol is itself an alias
2. Follow the chain recursively until reaching the original definition
3. Generate intermediate bindings if needed for clarity
4. Example: `A.foo → B.bar → C.baz` results in proper chain resolution

### Phase 4: Cross-Module Reference Tracking

**Problem**: References between modules are not properly tracked, leading to undefined symbols. Multiple modules may use identical symbol names with different meanings, requiring sophisticated deduplication.

**Solution** (inspired by Rolldown's SymbolRefDb):

1. Build a global symbol table during the analysis phase
2. Track every symbol definition and its visibility scope
3. Generate proper imports or assignments for cross-module references
4. Use unique symbol IDs to handle name collisions

**Implementation Components**:

```rust
// Adapted from Rolldown's index-based approach
struct GlobalSymbolTable {
    // Generate unique IDs for each symbol
    next_symbol_id: u32,
    // symbol_id -> SymbolInfo
    symbols: Vec<SymbolInfo>,
    // (module, name) -> symbol_id for fast lookup
    symbol_lookup: HashMap<(String, String), u32>,
    // Track symbol usage across modules
    symbol_refs: HashMap<u32, Vec<SymbolRef>>,
}

struct SymbolInfo {
    id: u32,
    defining_module: String,
    original_name: String,
    renamed_name: String,
    visibility: SymbolVisibility,
    is_reassigned: bool, // Track if symbol is mutated
}

struct SymbolRef {
    referencing_module: String,
    reference_type: RefType,
    span: Span, // Location in source
}

enum RefType {
    Read,
    Write,
    Import,
    Export,
}

enum SymbolVisibility {
    Public,  // Exported explicitly
    Private, // Module-local
    Imported { from: String, original: String },
}
```

**Deduplication Strategy**:

1. Each symbol gets a unique ID regardless of name
2. When identical names exist in different modules, they get different IDs
3. Reference resolution uses IDs, not names
4. Final code generation maps IDs back to appropriate renamed symbols

### Phase 5: Import Statement Generation

**Problem**: The bundler needs to generate proper import statements or assignments for all referenced symbols.

**Solution**:

1. After all modules are processed, analyze unresolved references
2. Generate appropriate binding statements based on the symbol table
3. Place these at the correct scope level

**Algorithm**:

```
1. Collect all unresolved symbols per module
2. For each unresolved symbol:
   a. Look up in global symbol table
   b. Determine if it needs import or direct assignment
   c. Generate appropriate statement:
      - Direct assignment: `helper_validate = validate_5`
      - Module reference: `UserLogger = sys.modules['models.user'].Logger_1`
   d. Insert at module initialization or global scope
```

### Phase 6: Circular Import Resolution

**Problem**: The fixture has circular imports that need special handling. We must ensure circular imports don't disrupt normal module initialization when modules are imported but never invoked.

**Solution** (adapted from Rolldown's approach):

1. Detect circular import chains during dependency analysis using DFS
2. Mark modules involved in cycles for special handling
3. Use lazy initialization for circular dependencies
4. Ensure modules can be imported without execution side effects

**Implementation Strategy**:

```rust
struct CircularDependencyDetector {
    // Track module visitation state
    module_states: HashMap<String, VisitState>,
    // Store detected cycles
    cycles: Vec<Vec<String>>,
    // Modules that need wrapping due to cycles
    modules_needing_wrap: HashSet<String>,
}

enum VisitState {
    Unvisited,
    Visiting, // Currently in DFS stack
    Visited,
}

// Detection algorithm (from Rolldown)
fn detect_cycles(&mut self, module: &str, path: &mut Vec<String>) {
    self.module_states
        .insert(module.to_string(), VisitState::Visiting);
    path.push(module.to_string());

    for dependency in get_dependencies(module) {
        match self.module_states.get(&dependency) {
            Some(VisitState::Visiting) => {
                // Found cycle - extract it from path
                let cycle_start = path.iter().position(|m| m == &dependency).unwrap();
                self.cycles.push(path[cycle_start..].to_vec());
            }
            Some(VisitState::Visited) => continue,
            _ => self.detect_cycles(&dependency, path),
        }
    }

    path.pop();
    self.module_states
        .insert(module.to_string(), VisitState::Visited);
}
```

**Module Wrapping for Cycles**:

1. Modules in cycles get wrapped in initialization functions
2. Imports become lazy - only execute when actually used
3. Module-level code that doesn't define symbols can run immediately
4. Symbol definitions are deferred until first access

## Implementation Order

### Phase 1: Symbol Reference Database Foundation

- Implement Rolldown-inspired symbol tracking system
- Create unique ID generation for all symbols
- Build basic symbol table infrastructure

### Phase 2: AST Scanner Enhancement

- Single-pass AST scanner for symbol collection
- Track imports, exports, and references
- Collect span information for precise transformations

### Phase 3: Type Annotation Support

- Implement type annotation visitor
- Apply renames to function signatures and class definitions
- Handle complex type expressions

### Phase 4: Import Resolution Pipeline

- Build import matcher following Rolldown patterns
- Handle relative imports and re-exports
- Create facade symbols for missing exports

### Phase 5: Alias Chain Resolution

- Implement transitive alias tracking
- Handle multi-level re-exports
- Generate proper binding chains

### Phase 6: Circular Dependency Handling

- Port Rolldown's DFS-based cycle detection
- Implement module wrapping for cycles
- Ensure lazy initialization support

## Testing Strategy

### Unit Tests

- Test each component in isolation
- Mock complex scenarios
- Verify transformations are correct

### Integration Tests

- Start with simple naming conflicts
- Gradually increase complexity
- Use `comprehensive_ast_rewrite` as final validation

### Test Cases Priority

1. Simple type annotation renaming
2. Basic import in wrapper function
3. Single-level alias resolution
4. Multi-level alias chains
5. Cross-package imports
6. Circular import scenarios
7. Full comprehensive test

## Success Criteria

The implementation is successful when:

1. `comprehensive_ast_rewrite` fixture passes without errors
2. Bundled code executes correctly
3. All symbol references are resolved
4. No performance regression in bundling time
5. Other existing tests continue to pass

## Risk Mitigation

### Complexity Risk

- Break down into smaller, testable components
- Implement incrementally with tests
- Regular code reviews

### Performance Risk

- Profile symbol table operations
- Use efficient data structures
- Cache computed results

### Compatibility Risk

- Ensure changes don't break existing functionality
- Run full test suite after each phase
- Keep changes backward compatible

## Alternative Approaches

If full implementation proves too complex:

1. **Simplified Type Annotations**: Strip type annotations instead of renaming
2. **Import Restrictions**: Require specific import patterns
3. **Partial Resolution**: Handle 90% of cases, document limitations
4. **Two-Pass Bundling**: First pass for analysis, second for generation

## Key Design Decisions

### Why Rolldown's Architecture Works

1. **Index-based References**: Using u32 IDs instead of string names reduces memory usage and makes lookups O(1)
2. **Two-tier Symbol System**: Separating module-local from global symbols simplifies resolution
3. **Single-pass Scanning**: Collecting all information upfront enables better optimization
4. **Explicit Phases**: Clear separation between analysis, linking, and code generation

### Python-Specific Adaptations

1. **Dynamic Imports**: Unlike JS, Python's import system is dynamic - we need runtime fallbacks
2. **Module Attributes**: Python modules are objects with attributes, requiring different handling
3. **Circular Import Tolerance**: Python handles some circular imports gracefully - we must preserve this
4. **Type Annotations**: Python's optional typing requires special AST handling

## Conclusion

This implementation plan, enhanced with Rolldown's architectural insights, provides a robust approach to making the `comprehensive_ast_rewrite` fixture pass. The key improvements include:

1. **Chained alias resolution** that handles transitive imports across multiple modules
2. **Symbol deduplication** using unique IDs to prevent name collision issues
3. **Circular import handling** that preserves Python's import semantics while avoiding initialization problems

By adapting Rolldown's battle-tested patterns to Python's specific needs, we can build a bundler that handles even the most complex naming conflicts and import patterns correctly.
