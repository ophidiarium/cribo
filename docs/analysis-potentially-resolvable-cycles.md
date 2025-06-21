# Analysis: Potentially Resolvable Import Cycles in Python Bundling

## The Problem

The bundler detects a circular dependency in the `cross-package-mixed-import` fixture:

```
Cycle 1: core → core.database → core.database.connection
  Type: ClassLevel
  Suggestion: Move imports inside functions to enable lazy loading
```

However, Python successfully executes this code without any import errors. This raises the question: what would it take for the bundler to handle "potentially resolvable" cycles like Python does?

## Understanding the Import Cycle

### The Import Chain

1. **`core/__init__.py`**:
   - Line 33: `from .database import connect as db_connect`

2. **`core/database/__init__.py`**:
   - Line 28: `from .connection import connect, get_connection_info`
   - Line 31: `from .. import _initialized`

3. **`core/database/connection.py`**:
   - Line 17: `from .. import CORE_MODEL_VERSION`
   - Line 20: `from . import _registered_types, validate_db_name`

### Why Python Allows This

Python handles this cycle successfully because of its import mechanism:

1. **Partial Module Objects**: When Python starts importing a module, it immediately creates a module object and adds it to `sys.modules`, even before the module is fully executed.

2. **Import Order Matters**: The specific sequence of imports allows the cycle to resolve:
   - `core/__init__.py` starts executing
   - It defines `CORE_MODEL_VERSION` (line 8) before importing from `.database` (line 33)
   - When `core.database.connection` imports `CORE_MODEL_VERSION` from parent, it's already defined

3. **Lazy Attribute Access**: Python doesn't validate that imported names exist until they're actually accessed. The import statement itself succeeds as long as the module object exists in `sys.modules`.

## Current Bundler Classification

The bundler classifies this as a `ClassLevel` circular dependency because:

1. The imports happen at module level (not inside functions)
2. The imported values are used in module-level code (e.g., `CONNECTION_METADATA` dictionary)
3. It's not a simple parent-child package relationship (involves 3 modules)

## What Would It Take to Handle This?

### 1. More Sophisticated Cycle Analysis

The bundler would need to analyze not just *where* imports occur, but *what* is being imported and *when* it's used:

```rust
// Pseudo-code for enhanced analysis
fn analyze_import_safety(cycle: &[ModuleId]) -> CycleSafety {
    // Track what each module imports from others in the cycle
    let mut import_map = HashMap::new();

    // Track when each name is defined vs when it's imported
    let mut definition_order = Vec::new();

    for module in cycle {
        // Parse AST to find:
        // - Order of statements
        // - What names are defined before imports
        // - What names are imported from cycle modules
        // - Whether imported names are used immediately or deferred
    }

    // Determine if cycle is safe based on definition order
    if all_imports_resolve_in_order(&import_map, &definition_order) {
        CycleSafety::Safe
    } else {
        CycleSafety::RequiresTransformation
    }
}
```

### 2. Preserve Python's Import Semantics

The bundler would need to maintain Python's partial module behavior:

```python
# Instead of current approach, generate something like:
import sys

# Create module objects first
_mod_core = types.ModuleType('core')
_mod_core_database = types.ModuleType('core.database')
_mod_core_database_connection = types.ModuleType('core.database.connection')

# Register in sys.modules immediately
sys.modules['core'] = _mod_core
sys.modules['core.database'] = _mod_core_database
sys.modules['core.database.connection'] = _mod_core_database_connection

# Then execute module bodies in dependency order
# This allows circular imports to find partially-initialized modules
```

### 3. Statement-Level Dependency Tracking

Track dependencies at the statement level, not just module level:

```rust
struct StatementDependency {
    module: ModuleId,
    statement_index: usize,
    imports_from: Vec<(ModuleId, Vec<String>)>,
    defines: Vec<String>,
    uses: Vec<String>,
}
```

### 4. Execution Order Simulation

Simulate Python's execution order to verify cycle safety:

```rust
fn simulate_import_execution(modules: &[ParsedModule]) -> Result<ExecutionOrder> {
    let mut module_states = HashMap::new();
    let mut execution_queue = VecDeque::new();

    // Start with entry module
    execution_queue.push_back(entry_module);

    while let Some(current) = execution_queue.pop_front() {
        // Execute statements in order
        for stmt in current.statements {
            match stmt {
                Import(module) => {
                    if !module_states.contains_key(module) {
                        // Create partial module state
                        module_states.insert(module, PartialModule::new());
                        execution_queue.push_back(module);
                    }
                }
                Define(name) => {
                    module_states.get_mut(current).add_definition(name);
                } // ... handle other statement types
            }
        }
    }

    Ok(ExecutionOrder::from(module_states))
}
```

### 5. Transform Only When Necessary

Instead of rejecting all class-level cycles, transform only those that would actually fail:

```python
# Original (potentially problematic):
from .submodule import something
CONSTANT = something.value  # Immediate use

# Transformed (when necessary):
def _get_something():
    from .submodule import something
    return something
CONSTANT = None  # Placeholder
def _init_module():
    global CONSTANT
    CONSTANT = _get_something().value
```

## Challenges

1. **Complexity**: Accurately simulating Python's import mechanism is complex
2. **Performance**: Deep analysis of import sequences could slow down bundling
3. **Edge Cases**: Python has many import edge cases (conditional imports, try/except imports, etc.)
4. **Maintenance**: Would need to track changes in Python's import behavior across versions

## Recommendation

For now, the bundler's conservative approach (treating these as errors) is reasonable because:

1. It ensures the bundled output will work reliably
2. It encourages cleaner import structures
3. Manual resolution (moving imports to functions) is straightforward

However, supporting "potentially resolvable" cycles would make the bundler more compatible with existing Python codebases that rely on Python's permissive import system.
