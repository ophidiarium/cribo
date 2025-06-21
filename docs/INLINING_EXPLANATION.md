# Understanding "Incorrect Inlining" in Python Bundlers

## What is Module Inlining?

When a Python bundler "inlines" a module, it takes the module's code and embeds it directly into the bundle, rather than keeping it as a separate module. This is a performance optimization, but it must be done carefully.

## Example: The Difference

### Original Code Structure

```python
# models/base.py
def initialize():
    return "initialized"

# services/auth/manager.py
from models import base
result = base.initialize()
```

### Approach 1: Wrapper Module (Correct for module imports)

```python
# Bundled code
def __cribo_init_models_base():
    module = types.ModuleType('models.base')
    def initialize():
        return "initialized"
    module.initialize = initialize
    sys.modules['models.base'] = module
    return module

# In services.auth.manager:
base = sys.modules['models.base']  # Import the MODULE
result = base.initialize()
```

### Approach 2: Inlined (What was happening before)

```python
# Bundled code - everything at top level
def initialize_models_base():
    return "initialized"

# In services.auth.manager:
base = types.SimpleNamespace(
    initialize=initialize_models_base  # Direct reference to function
)
result = base.initialize()
```

## Why Inlining `from models import base` Was Incorrect

### 1. Semantic Difference

- `from models import base` means "import the MODULE named base"
- The module should exist in `sys.modules`
- Other code might do `import models.base` or check `sys.modules['models.base']`
- Inlining breaks these expectations

### 2. Python Import Semantics

```python
# This should work:
from models import base
import sys
assert 'models.base' in sys.modules  # ❌ Fails if inlined!
assert sys.modules['models.base'] is base  # ❌ Fails if inlined!

# Dynamic imports should work:
importlib.import_module('models.base')  # ❌ Fails if inlined!
```

### 3. Module State and Identity

```python
# models/base.py
_counter = 0
def increment():
    global _counter
    _counter += 1
    return _counter

# If inlined, module state is lost and shared incorrectly
# Multiple imports might create multiple counters!
```

## What We Didn't Notice

### 1. The Tests Were Too Simple

The tests only checked if the code executed and produced the right output. They didn't check:

- Whether modules existed in `sys.modules`
- Whether module identity was preserved
- Whether dynamic imports would work
- Whether module state was correctly isolated

### 2. Hidden Coupling

The code generator "knew" that certain modules would be inlined and generated code assuming those symbols would exist globally. This created hidden coupling between the inlining decision and the code generation.

### 3. It "Worked" But Was Semantically Wrong

```python
# What the user wrote:
from models import base  # Import a MODULE

# What actually happened (inlined):
base = types.SimpleNamespace(...)  # Create a fake object

# The code ran, but 'base' wasn't really the module!
```

## The Correct Behavior

### When to Inline

Only inline when importing VALUES from a module:

```python
from models.base import initialize  # Importing a FUNCTION (value)
# Can inline: initialize = initialize_models_base
```

### When NOT to Inline

Never inline when importing the MODULE itself:

```python
from models import base  # Importing a MODULE
import models.base      # Importing a MODULE
# Must preserve as real module in sys.modules
```

## Why This Matters

### 1. Correctness

Python code has expectations about how imports work. Breaking these can cause subtle bugs in user code.

### 2. Compatibility

Third-party tools, debuggers, and inspection utilities expect modules to behave like modules.

### 3. Future Features

Supporting dynamic imports, lazy loading, or other advanced features requires proper module semantics.

## The Real Achievement

By adding the `is_namespace_imported` check, we:

1. **Fixed the semantic incorrectness** - modules are now real modules
2. **Exposed the hidden coupling** - the code generator was assuming inlining
3. **Made the bundler more correct** - it now respects Python's import semantics

The "regression" is actually progress - we're finding and fixing incorrect assumptions that happened to work but were fundamentally wrong.

## Analogy

It's like having a calculator that computed `2 + 2 = 4` by coincidence (maybe it always returned 4), not because it actually did addition. When you fix it to do real addition, other calculations that depended on it always returning 4 would "break" - but they were wrong to begin with!
