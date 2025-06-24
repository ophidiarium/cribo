# importlib and sys.modules: Complete Integration Guide

## Overview

This document describes how Python's `importlib.import_module()` interacts with `sys.modules`, including edge cases and implications for bundlers like Cribo.

## Key Findings

### 1. importlib Registers Modules in sys.modules

`importlib.import_module()` behaves **exactly** like regular import statements when it comes to `sys.modules` registration:

```python
import mymodule                           # Registers as sys.modules['mymodule']
importlib.import_module('mymodule')       # Also registers as sys.modules['mymodule']

# They return the same object
assert mymodule is sys.modules['mymodule']
assert importlib.import_module('mymodule') is sys.modules['mymodule']
```

### 2. Parent Modules Are Automatically Registered

When importing nested modules, all parent packages are automatically registered:

```python
# This single import:
importlib.import_module('package.submodule.leaf')

# Registers ALL of these in sys.modules:
# - 'package'
# - 'package.submodule'  
# - 'package.submodule.leaf'
```

Each parent module is executed (its `__init__.py` runs) and becomes accessible as an attribute of its parent.

### 3. Module Names Can Be ANY String

Unlike regular `import` statements which require valid Python identifiers, `importlib.import_module()` accepts **any string** as a module name:

#### Reserved Keywords

```python
# SyntaxError: import class
# But this works:
class_module = importlib.import_module('class')
sys.modules['class']  # Valid key!
```

#### Names Starting with Numbers

```python
# SyntaxError: import 123_module
# But this works:
module = importlib.import_module('123_module')
sys.modules['123_module']  # Valid key!
```

#### Names with Special Characters

```python
# All of these work with importlib:
importlib.import_module('my-module')      # Hyphens
importlib.import_module('my module')      # Spaces!
importlib.import_module('module@v2')      # Special chars
importlib.import_module('data.2023.jan')  # Numbers in path

# They all register in sys.modules with exact names:
sys.modules['my-module']
sys.modules['my module']
sys.modules['module@v2']
sys.modules['data.2023.jan']
```

### 4. Deduplication Is By Exact String Match

Python deduplicates modules based on their **exact** name in `sys.modules`:

```python
# First import executes the module
mod1 = importlib.import_module('my-module')  # Executes my-module.py

# Second import returns cached module
mod2 = importlib.import_module('my-module')  # Returns from sys.modules
assert mod1 is mod2  # Same object!

# But different names = different modules
import submodule                     # sys.modules['submodule']
from pkg import submodule           # sys.modules['pkg.submodule']
# These are TWO different module objects!
```

### 5. Module Access Patterns

Once a module with an invalid Python identifier is imported:

```python
# Import a module named 'class'
class_mod = importlib.import_module('class')

# Can access via:
class_mod.some_function()                    # Direct reference
sys.modules['class'].some_function()         # sys.modules lookup
importlib.import_module('class').some_function()  # Re-import

# CANNOT access via:
# import class  # SyntaxError!
# from class import something  # SyntaxError!
```

## Complete Example

Here's a comprehensive example showing all the behaviors:

```python
import sys
import importlib
import os

# Create test structure
os.makedirs('test/sub-pkg', exist_ok=True)

# Module with reserved name
with open('test/class.py', 'w') as f:
    f.write("value = 'I am class.py'")

# Module in package with hyphen
with open('test/sub-pkg/__init__.py', 'w') as f:
    f.write("")
with open('test/sub-pkg/for.py', 'w') as f:
    f.write("value = 'I am for.py in sub-pkg'")

# Import modules with problematic names
class_mod = importlib.import_module('test.class')
for_mod = importlib.import_module('test.sub-pkg.for')

# Check sys.modules
print(sys.modules['test.class'])         # <module 'test.class' from 'test/class.py'>
print(sys.modules['test.sub-pkg.for'])   # <module 'test.sub-pkg.for' from 'test/sub-pkg/for.py'>

# Access values
print(class_mod.value)  # 'I am class.py'
print(for_mod.value)    # 'I am for.py in sub-pkg'
```

## Implications for Bundlers

### 1. Module Name Handling

Bundlers must handle module names as **arbitrary strings**, not just valid Python identifiers:

```python
# These are all valid module names for importlib:
'class'
'123_start'
'my-module'
'my module'
'pkg.sub-pkg.class'
'data.2023.reports'
```

### 2. Deduplication Strategy

Deduplication must be based on the **exact module name** as it appears in `sys.modules`:

```python
# Different names = different modules (even if same file!)
'utils'              # From: import utils
'package.utils'      # From: from package import utils
'package.sub.utils'  # From: from package.sub import utils
```

### 3. Import Detection

When detecting `importlib.import_module()` calls, bundlers must:

1. **Handle string literals**: `importlib.import_module('my.module')`
2. **Consider parent modules**: Importing 'a.b.c' also imports 'a' and 'a.b'
3. **Preserve exact names**: Don't normalize 'my-module' to 'my_module'

### 4. Bundling Edge Cases

Special considerations for modules with invalid Python identifiers:

```python
# Original code:
data_mod = importlib.import_module('data-2023')
processor = data_mod.process_data

# Cannot be rewritten as:
import data-2023  # SyntaxError!

# Must maintain importlib usage or use sys.modules:
data_mod = sys.modules['data-2023']  # After ensuring it's loaded
```

## Test Fixtures

The `importlib_deduplication` fixture demonstrates all these behaviors:

- `test_sys_modules.py` - Basic importlib and sys.modules interaction
- `test_parent_registration.py` - Parent module auto-registration
- `test_edge_cases.py` - Modules with invalid Python identifiers
- `test_complete_picture.py` - Comprehensive demonstration

## Summary

1. **`importlib.import_module()` = regular import + sys.modules registration**
2. **Module names can be ANY string** (not just valid identifiers)
3. **Deduplication by exact string match** in sys.modules
4. **Parent modules auto-registered** with proper attributes
5. **Same file can exist under multiple names** in sys.modules

This flexibility makes Python's import system more powerful but also more complex for static analysis and bundling tools.
