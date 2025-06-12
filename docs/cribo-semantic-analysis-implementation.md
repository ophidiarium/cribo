# Cribo Semantic Analysis Implementation

## Overview

This document describes the semantic analysis implementation in Cribo, which provides advanced Python code understanding capabilities for the bundling process. The semantic analysis system enables proper handling of variable scoping, name resolution, module-level globals lifting, and namespace imports resolution.

## Architecture

### Core Components

#### 1. SemanticBundler (`semantic_bundler.rs`)

The `SemanticBundler` is the main orchestrator for semantic analysis, built on top of Ruff's semantic analysis infrastructure.

**Key Structures:**

```rust
pub struct SemanticBundler<'a> {
    db: SemanticDb<'a>,
    module_globals: FxIndexMap<ModuleId, ModuleGlobalInfo>,
    symbol_registry: SymbolRegistry,
    // ... other fields
}

pub struct ModuleGlobalInfo {
    pub module_level_vars: FxIndexSet<String>,
    pub global_declarations: FxIndexMap<String, Vec<TextRange>>,
    pub global_reads: FxIndexMap<String, Vec<TextRange>>,
    pub global_writes: FxIndexMap<String, Vec<TextRange>>,
    pub functions_using_globals: FxIndexSet<String>,
    pub module_name: String,
}
```

**Responsibilities:**

- Tracks module-level variable definitions
- Identifies global statements and their usage patterns
- Records which functions use global declarations
- Manages symbol renaming to avoid conflicts

#### 2. GlobalUsageVisitor

A visitor that traverses the AST to collect information about global variable usage:

```rust
struct GlobalUsageVisitor<'a> {
    info: &'a mut ModuleGlobalInfo,
    current_function: Option<String>,
    in_global_scope: bool,
}
```

**Key Behaviors:**

- Tracks module-level variable assignments
- Records global declarations within functions
- Identifies reads and writes to global variables
- Maintains context about current scope (function vs module level)

### Integration with Code Generation

#### 1. Namespace Imports Handler (`code_generator.rs`)

The code generator now includes sophisticated namespace import detection and handling:

```rust
struct PythonCodeGenerator {
    namespace_imported_modules: FxIndexMap<String, FxIndexSet<String>>,
    // ... other fields
}
```

**Namespace Import Detection:**

The system identifies when modules are imported as namespaces (e.g., `from package import module`):

```rust
fn find_namespace_imported_modules(&mut self, module_asts: &FxIndexMap<String, ParsedModule>) {
    for (module_name, parsed_module) in module_asts {
        let namespace_imports = self.collect_namespace_imports(&parsed_module.ast);

        for imported_module in namespace_imports {
            self.namespace_imported_modules
                .entry(imported_module)
                .or_default()
                .insert(module_name.clone());
        }
    }
}
```

**Hybrid Inlining Approach:**

When a module is imported as a namespace, it uses a hybrid approach that combines inlining with namespace preservation:

```rust
fn inline_module_for_namespace(
    &mut self,
    module_name: &str,
    ast: ModModule,
    ctx: &mut InlineContext,
) -> Result<Vec<Stmt>>
```

**Example Transformation:**

```python
# Original: models/base.py
class BaseModel:
    pass

# Original: main.py
from models import base
model = base.BaseModel()

# After transformation:
class BaseModel_models_base:
    pass
base = types.SimpleNamespace()
base.BaseModel = BaseModel_models_base
model = base.BaseModel()
```

#### 2. GlobalsLifter (`code_generator.rs`)

The `GlobalsLifter` transforms module-level variables that are referenced with global statements:

```rust
struct GlobalsLifter<'a> {
    global_info: &'a ModuleGlobalInfo,
}
```

**Transformation Process:**

1. **Identify Variables to Lift**: Variables that are both module-level and referenced in global statements
2. **Generate Lifted Names**: Create unique names like `__cribo_module_var` for lifted variables
3. **Create Declarations**: Generate top-level assignments for lifted variables
4. **Transform Functions**:
   - Add initialization statements mapping local names to lifted globals
   - Rewrite global statements to use lifted names
   - Transform variable references within functions

**Example Transformation:**

```python
# Original module code
result = "module_result"

def process():
    global result
    result = "processed"
    return result

# After lifting
__cribo_module_result = "module_result"

def process():
    result = __cribo_module_result  # Added initialization
    global __cribo_module_result
    __cribo_module_result = "processed"
    return __cribo_module_result
```

#### 2. globals() Transformation

When modules are wrapped, `globals()` calls need special handling to maintain correct namespace access:

```rust
fn transform_globals_in_expr(expr: &mut Expr) {
    match expr {
        Expr::Call(call_expr) => {
            if is_globals_call(call_expr) {
                // Transform globals() to module.__dict__
                *expr = create_module_dict_access();
            }
        } // ... recursive handling
    }
}
```

**Example:**

```python
# Original
data = globals().get('some_var')

# Transformed in wrapped module
data = module.__dict__.get('some_var')
```

### Symbol Resolution and Renaming

#### 1. Variable Name Conflict Resolution

The system handles conflicts between:

- Module-level variables across different modules
- Function/class names that might clash
- Variables referenced in global statements
- Symbols from namespace-imported modules

**Resolution Strategy:**

1. **Detection**: Identify all symbols that could conflict
2. **Renaming**: Generate unique names using module-qualified suffixes
3. **Mapping**: Maintain registry of original → renamed mappings
4. **Application**: Apply renames consistently throughout the AST

**Namespace Import Symbol Renaming:**

For namespace imports, symbols get module-qualified names:

```python
# Original: helpers.py
def format_data(data):
    return f"Formatted: {data}"

# When imported as namespace: from utils import helpers
# Becomes:
def format_data_utils_helpers(data):
    return f"Formatted: {data}"

helpers = types.SimpleNamespace()
helpers.format_data = format_data_utils_helpers
```

#### 2. Scoping Rules

The implementation respects Python's scoping rules:

```python
# Module level
Logger = "module"  # This gets renamed to Logger_1

def function():
    # Function level - shadows module level
    Logger = "local"  # This is NOT renamed
    return Logger  # Returns "local"

# After function, module level is accessible
print(Logger)  # This references the renamed Logger_1
```

**Key Insight**: Renames are only applied at module scope, not within function bodies where local variables shadow module-level ones.

## Implementation Details

### Module Analysis Flow

1. **Parse Module**: Use Ruff's parser to create AST
2. **Build Semantic Model**: Create bindings and scope information
3. **Detect Namespace Imports**:
   ```rust
   self.find_namespace_imported_modules(&modules);
   ```
4. **Analyze Global Usage**:
   ```rust
   let mut visitor = GlobalUsageVisitor::new(&mut global_info);
   visitor.visit_body(&module.body);
   ```
5. **Register Symbols**: Track all module-level definitions
6. **Apply Transformations**:
   - Lift globals and resolve conflicts
   - Apply namespace hybrid inlining for namespace imports
   - Rename symbols with module-qualified names

### Test Framework Integration

The implementation includes comprehensive testing:

1. **Fixture Validation**: Tests run original Python code first to ensure it's valid
2. **Output Comparison**: Bundled output must match original execution
3. **Pretty Assertions**: Clear diffs when outputs don't match

```rust
// Run original fixture
let original_output = Command::new(&python_cmd)
    .arg(path)
    .current_dir(fixture_dir)
    .output()
    .expect("Failed to execute original fixture");

// Compare outputs
assert_eq!(
    original_stdout, 
    bundled_stdout,
    "\nBundled output differs from original for fixture '{}'",
    fixture_name
);
```

### Edge Cases Handled

1. **Nested Global Access**:
   ```python
   def outer():
       def inner():
           global x
           x = 10
   ```

2. **Complex Import Scenarios**:
   - Cross-module globals usage
   - Relative imports with global declarations
   - Module-level imports that shadow globals
   - Namespace imports (`from package import module`)
   - Re-exports in `__init__.py` files

3. **Special Function Handling**:
   - `globals()` calls in various contexts
   - Dynamic attribute access on module objects
   - Comprehensions and lambda functions
   - F-strings with renamed symbol references

4. **Namespace Import Edge Cases**:
   - Modules imported both directly and as namespaces
   - Circular dependencies with namespace imports
   - Namespace imports with aliasing
   - Global variables in namespace-imported modules

## Performance Considerations

1. **Single-Pass Analysis**: Module analysis happens once during bundling
2. **Efficient Data Structures**: Uses `FxIndexMap` and `FxIndexSet` for better performance
3. **Lazy Transformation**: Only transform when globals are actually used

## Future Enhancements

1. **Enhanced Diagnostics**: Better error messages for global conflicts and namespace import issues
2. **Optimization**: Minimize number of lifted variables and namespace objects
3. **Dynamic Import Support**: Handle dynamic imports with namespace preservation

## Configuration

The semantic analysis features are automatic and require no configuration. They activate when:

- Module-level variables are defined
- Global statements reference those variables
- Modules are imported as namespaces (`from package import module`)
- The bundler needs to maintain correct Python semantics

The namespace imports feature specifically activates when detecting `ImportFrom` statements that import module names rather than specific symbols.

## Testing

The implementation is thoroughly tested with:

- Unit tests for individual components
- Integration tests with complex fixtures
- The `comprehensive_ast_rewrite` fixture demonstrating all features

Key test fixture that validates the implementation:

```
crates/cribo/tests/fixtures/bundling/comprehensive_ast_rewrite/
├── main.py                    # Entry point with conflicts
├── models/
│   ├── base.py               # Base model with relative imports
│   └── user.py               # User model with global usage
├── services/
│   └── auth/
│       └── manager.py        # Complex global patterns
└── core/
    ├── utils/
    │   └── helpers.py        # Utility functions
    └── database/
        └── connection.py     # Database module
```

This fixture tests:

- Multiple naming conflicts across modules
- Global variable lifting and access
- Cross-module imports and dependencies
- Complex scoping scenarios
- globals() function transformation
