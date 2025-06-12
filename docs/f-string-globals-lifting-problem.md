# F-String Globals Lifting Problem

## Problem Statement

When module-level variables are lifted to global scope with renamed identifiers (e.g., `result` → `__cribo_module_result`), f-strings that reference these variables become broken because f-strings perform name lookup at runtime in the local scope.

### Example of the Problem

```python
# Original module code
result = "module_result"

def process():
    global result
    result = "processed"
    return f"Result is: {result}"  # This references 'result'

# After globals lifting
__cribo_module_result = "module_result"

def process():
    result = __cribo_module_result  # Initialization
    global __cribo_module_result
    __cribo_module_result = "processed"
    return f"Result is: {result}"  # ❌ PROBLEM: Still references 'result'
    # But 'result' is now a local variable that never gets updated!
```

The issue is that the f-string still references `result`, but after the transformation:

- `result` is initialized to the value of `__cribo_module_result` at function entry
- Updates happen to `__cribo_module_result` (the global)
- The local `result` variable remains unchanged
- The f-string reads the stale local value

## Why This Happens

1. **F-strings compile to bytecode** that performs name lookups at runtime
2. **Variable resolution order**: Local scope is checked before global scope
3. **Our transformation creates a local shadow**: The initialization `result = __cribo_module_result` creates a local variable that shadows the global

## Proposed Solutions

### Solution 1: F-String AST Transformation (Recommended)

Transform f-strings to use the lifted global names directly.

**Implementation**:

```rust
fn transform_fstring_for_lifted_globals(
    expr: &mut ExprFString,
    lifted_names: &FxIndexMap<String, String>,
    in_function_with_globals: Option<&FxIndexSet<String>>,
) {
    for part in &mut expr.parts {
        if let FStringPart::FString(fstring) = part {
            for element in &mut fstring.elements {
                if let FStringElement::Expression(expr_element) = element {
                    // Transform the expression inside the f-string
                    transform_expr_for_lifted_globals(
                        &mut expr_element.expression,
                        lifted_names,
                        in_function_with_globals,
                    );
                }
            }
        }
    }
}
```

**Example transformation**:

```python
# Before
f"Result is: {result}"

# After  
f"Result is: {__cribo_module_result}"
```

**Pros**:

- Clean and direct solution
- Maintains f-string performance
- No runtime overhead
- Transparent to users

**Cons**:

- Requires deep AST traversal
- Must handle nested expressions in f-strings
- Complex expressions need careful handling

### Solution 2: Update Local Variables (Alternative)

Instead of just initializing local variables, continuously update them when globals change.

**Example**:

```python
def process():
    result = __cribo_module_result  # Initialization
    global __cribo_module_result
    __cribo_module_result = "processed"
    result = __cribo_module_result  # ✅ Update local too
    return f"Result is: {result}"  # Now this works
```

**Pros**:

- No f-string transformation needed
- Simpler AST manipulation

**Cons**:

- Requires tracking all assignment locations
- Performance overhead (double assignments)
- More complex code generation
- Can miss updates in nested scopes

### Solution 3: Avoid Local Shadow Variables

Don't create local initialization variables; use only the lifted globals.

**Example**:

```python
def process():
    global __cribo_module_result
    __cribo_module_result = "processed"
    return f"Result is: {__cribo_module_result}"  # Direct reference
```

**Pros**:

- Simplest approach
- No shadowing issues

**Cons**:

- Requires renaming ALL occurrences of the variable
- Changes code semantics (always uses global)
- May break code that expects local shadowing

### Solution 4: Runtime Wrapper (Not Recommended)

Create a wrapper object that synchronizes local and global values.

```python
class GlobalRef:
    def __init__(self, global_name):
        self.global_name = global_name
    
    def __str__(self):
        return str(globals()[self.global_name])

def process():
    result = GlobalRef('__cribo_module_result')
    global __cribo_module_result
    __cribo_module_result = "processed"
    return f"Result is: {result}"  # Works via __str__
```

**Pros**:

- No AST transformation of f-strings needed
- Handles all string formatting cases

**Cons**:

- Significant performance overhead
- Changes variable types
- Complex implementation
- May break type checking

## Recommended Approach

**Solution 1 (F-String AST Transformation)** is the recommended approach because:

1. **Correctness**: Directly addresses the root cause
2. **Performance**: No runtime overhead
3. **Compatibility**: Preserves Python semantics
4. **Maintainability**: Clear transformation logic

## Implementation Plan

1. **Extend `transform_expr_for_lifted_globals`** to handle `ExprFString`
2. **Process FString elements**: Transform expressions within f-string braces
3. **Handle complex cases**:
   - Nested f-strings
   - Format specifications (e.g., `{value:.2f}`)
   - Conversion flags (e.g., `{value!r}`)
   - Expressions with attribute access (e.g., `{obj.attr}`)

## Test Cases

### Basic F-String

```python
# Input
result = "test"
def func():
    global result
    return f"Value: {result}"

# Expected Output
__cribo_module_result = "test"
def func():
    result = __cribo_module_result
    global __cribo_module_result
    return f"Value: {__cribo_module_result}"
```

### Complex F-String Expression

```python
# Input
counter = 0
def func():
    global counter
    counter += 1
    return f"Count: {counter:03d} (hex: {counter:#x})"

# Expected Output  
__cribo_module_counter = 0
def func():
    counter = __cribo_module_counter
    global __cribo_module_counter
    __cribo_module_counter += 1
    return f"Count: {__cribo_module_counter:03d} (hex: {__cribo_module_counter:#x})"
```

### Nested Expression in F-String

```python
# Input
values = [1, 2, 3]
def func():
    global values
    return f"Sum: {sum(values)}, Max: {max(values)}"

# Expected Output
__cribo_module_values = [1, 2, 3]
def func():
    values = __cribo_module_values
    global __cribo_module_values
    return f"Sum: {sum(__cribo_module_values)}, Max: {max(__cribo_module_values)}"
```

## Conclusion

The f-string transformation approach provides the cleanest solution to the globals lifting problem. It maintains Python semantics while ensuring that f-strings correctly reference the lifted global variables. The implementation should be added to the existing AST transformation pipeline in the `transform_expr_for_lifted_globals` function.
