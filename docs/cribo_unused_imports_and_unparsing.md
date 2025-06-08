# Implementing Unused Import Detection (Ruff F401) and AST Unparsing in Cribo

## Introduction

This guide outlines how to detect and fix unused imports in a single Python file, using an AST-based approach inspired by Ruff’s **F401** rule (unused imports). Ruff’s implementation of F401 is derived from the Pyflakes linter, which means it uses Python’s AST and a scope-based model to determine if an imported name is ever used. We will cover how to parse Python code into an AST (using RustPython or CPython’s AST), how to track scopes and bindings to find unused imports, and how to handle special cases such as `__all__` exports, redundant aliases, shadowed imports, side-effect imports, `__future__` imports, and wildcard imports. We also explain how to construct minimal edits (or use AST unparsing) to safely remove or mark unused imports. By following this guide, another AI agent or tool should be able to implement similar logic in **Cribo** without having to consult Ruff’s source code directly.

_(Why detect unused imports?)_ Unused imports slow down imports at runtime and can introduce unwanted dependencies or import cycles. They clutter the code and add cognitive load. Automating their detection and removal helps keep code clean and efficient.

## Parsing Python Code to an AST

The first step is to parse the Python source file into an Abstract Syntax Tree (AST). Ruff uses a custom parser (based on RustPython) for speed, but you can achieve the same result with Python’s built-in `ast` module or RustPython’s AST, since both conform to Python’s grammar. The goal is to obtain a syntax tree where import statements and identifiers can be analyzed.

**Using CPython’s AST (Python example):**

```python
import ast
from pathlib import Path

code = Path("example.py").read_text()
tree = ast.parse(code, filename="example.py")
```

This produces an AST `tree` with nodes like `ast.Import` and `ast.ImportFrom` for import statements, and `ast.Name` for identifiers. In Rust or other environments, you’d use the equivalent parser (e.g., RustPython’s parser to get an AST struct). The key is that we will traverse this AST to collect information about imports and name usage.

## Scope and Binding Tracking (Pyflakes Model)

To detect unused imports, we need to track **which names are introduced (bound)** by import statements and whether they are **ever used** in the code. This requires understanding Python’s scoping rules. Ruff/Pyflakes simulate a simplified **symbol table**:

- **Scopes:** There is a global (module-level) scope for the file, and local scopes for each function or class definition. Imports at the top level define names in the global scope; imports inside a function define names in that function’s local scope.
- **Name binding:** An import statement binds one or more names in the current scope. For example:
  - `import math` binds the name `math` in the scope.
  - `from utils import foo as bar` binds the name `bar` in the scope.
  - If multiple names are imported (e.g., `import os, sys` or `from utils import foo, bar`), each is a binding.
- **References:** Any occurrence of a name in an expression (AST `Name` node in **Load** context) is a reference to some binding in the same or an enclosing scope.

The strategy is:

1. **Collect bindings:** Walk the AST to record all imported names and other definitions. For each import, note the name, its origin (module and original name), and the line number (for reporting and fixes).
2. **Track usage:** As you traverse or analyze the AST, mark an import as “used” if there is any reference to that name in the code (after its import).
3. **Detect shadowing:** Also note if an import’s name gets **shadowed** by a later definition in the same scope (e.g., a variable assignment or loop variable of the same name). Shadowing an import can make it effectively unused (if it wasn’t used before being overshadowed).

By the end of analysis, any import binding that is not marked used (and not excused by special cases) is considered an **unused import**. This will correspond to a Ruff F401 violation (“`<name>` imported but unused”).

### Managing Scope Context

A simplified way to implement this is to maintain a stack of scope dictionaries while traversing the AST:

- When entering a new scope (on a `FunctionDef`, `AsyncFunctionDef`, or `ClassDef` node), push a new dictionary for that scope.
- When leaving the scope, check for any unused imports in that scope’s dictionary.

Each scope’s dictionary can map variable names to information about their definition: whether it’s an import, a function/variable, etc., and a flag if it’s used. For example:

```python
scope_stack = []  # stack of dicts: name -> {"type": ..., "used": bool, "node": ...}
```

When you see an `ast.Import` or `ast.ImportFrom`, for each alias:

- Add an entry in the current scope for the alias name, with type “import” and used=False initially.

When you see an `ast.Name` in a usage context (e.g., inside an expression):

- Determine which scope it belongs to. This is like Python’s name resolution: look in the current (innermost) scope; if not found and not declared global, look upward to outer scopes.
- If the name exists in some scope and it’s an import binding, mark it as used.

When you see an assignment to a name (e.g., `ast.Name` in a Store context, or a `for` loop target):

- If that name already exists as an **import binding** in the current scope and it hasn’t been used yet, then we have a shadowing situation:
  - If the shadowing occurs in a loop (for-loop or comprehension target), this corresponds to rule F402 (“Import `{name}` from line X shadowed by loop variable”). The fix is to remove the import or warn.
  - If the shadowing is by a simple assignment or function definition, this corresponds to rule F811 (“Redefinition of unused `{name}` from line X”).
  - In either case, mark the import as handled (so you don’t also flag F401).
- Otherwise, if the name is not already bound in this scope, create a new binding (e.g., a local variable) as usual.

After processing the entire AST, any remaining import binding still marked unused (and not shadowed) should be reported as an unused import (F401). See examples below.

### Handling Nested Scopes

If you import a module at the top level and only use it inside a function, that still counts as “used.” When resolving a name inside the function, if it’s not defined there, it will find the binding in the global scope and mark it used. Be sure that references in inner scopes mark imports in outer scopes as used.

## Special Cases and Exceptions

Ruff’s rules include several special-case handling to avoid false positives. We describe how to handle:

### `__all__` Exports

If a module defines `__all__`, names in that list are considered used (re-exported). For example:

```python
from .utils import Helper  # import used only for re-export
__all__ = ["Helper"]
```

Even if `Helper` isn’t otherwise referenced, it’s exempt from F401. Implementation:

1. When you encounter an assignment to `__all__`, if it’s a list or tuple of string literals, collect those names.
2. Before flagging unused imports, mark any import whose alias or name appears in `__all__` as used.

### Redundant Alias Imports (Re-exports)

An import of the form `from module import name as name` is a redundant alias – it indicates an intending re-export. Treat it as used:

```python
from mypackage.utils import Helper as Helper  # redundant alias
```

Check in the AST: if `alias.name == alias.asname`, consider it used and do not flag it unused.

### Shadowed Imports (F402 & F811)

If an import name is later redefined:

- **Loop variable shadowing (F402):**
  ```python
  import data  # line 1
  for data in items:  # line 2 shadows import
      ...
  ```
  The import is unused and shadowed by a loop variable. Report F402 at line 2: “import `data` from line 1 shadowed by loop variable.” Remove the import (line 1).

- **Assignment/definition shadowing (F811):**
  ```python
  import os  # line 1
  os = "system"  # line 2
  ```
  The import is unused and redefined. Report F811 at line 2: “redefinition of unused `os` from line 1.” Remove the import on line 1.

Implementation:

1. On encountering a definition or assignment to name `N`, check if `N` is an import binding in the same scope and unused. If so:
   - If in a `for` loop head or comprehension, record F402.
   - Else record F811.
2. Mark the import as handled (so not flagged as F401).
3. Let subsequent references bind to the new definition.

### Allowed Unused Imports (Side-Effect Imports)

To handle imports intended for side effects, provide a configuration allowlist of module names or prefixes. If an import’s module matches an entry in this allowlist, treat it as used (skip F401). Optionally, also allow imports of the form `import X as _` (underscore alias) to be considered used, following recent Pyflakes changes.

Example allowlist usage:

```toml
[tool.cribo]
allowed_unused_imports = ["hvplot.pandas", "some_plugin"]
```

In code:

1. If an import’s module starts with any prefix in `allowed_unused_imports`, mark the import as used.

### `__future__` Imports

All `__future__` imports are implicitly used. If you see an `ImportFrom` node with `module=="__future__"`, skip adding those aliases to unused detection altogether.

### Wildcard Imports

Do not flag `from module import *` as unused, since you cannot determine which names are used. Optionally, warn about wildcard imports (analogous to Ruff’s F403) but do not remove them under F401.

## Constructing Fixes: Removing Unused Imports

After identifying unused imports, the next step is removing them from the source. Ruff creates **minimal edits** to the source code rather than rewriting the entire file. There are two main approaches:

1. **AST-based removal + Unparser**
2. **Direct text edits guided by AST node positions**

### 1. AST-based Removal + Unparser

Use a custom `NodeTransformer` on the AST to remove unused imports, then unparse back to source code with Python’s `ast.unparse()` (Python 3.9+). Note: this may not preserve original formatting or comments exactly.

**Example:**

```python
import ast
from pathlib import Path

class RemoveUnusedImports(ast.NodeTransformer):
    def __init__(self, unused_names):
        self.unused = set(unused_names)

    def visit_Import(self, node):
        # Filter out aliases that are unused
        new_names = []
        for alias in node.names:
            name = alias.asname or alias.name
            if name in self.unused:
                continue
            new_names.append(alias)
        if not new_names:
            return None  # Remove entire import statement
        node.names = new_names
        return node

    def visit_ImportFrom(self, node):
        # Keep __future__ imports
        if node.module == "__future__":
            return node
        new_names = []
        for alias in node.names:
            name = alias.asname or alias.name
            if name in self.unused:
                continue
            new_names.append(alias)
        if not new_names:
            return None  # Remove entire from-import
        node.names = new_names
        return node

# Usage:
code = Path("example.py").read_text()
tree = ast.parse(code, filename="example.py")

# Suppose `unused_names` is a list of names determined by analysis
transformer = RemoveUnusedImports(unused_names)
new_tree = transformer.visit(tree)
ast.fix_missing_locations(new_tree)

# Unparse back to code
new_code = ast.unparse(new_tree)

# Write back to file
Path("example_fixed.py").write_text(new_code)
```

**Pros:**

- Ensures resulting code is syntactically valid.
- Python handles comma placement and formatting of imports.

**Cons:**

- Comments and exact spacing may be lost or altered.
- Requires Python 3.9+ for `ast.unparse()`.

### 2. Direct Text Edits Guided by AST Node Positions

To preserve original comments and formatting, determine the **exact character spans** of unused import tokens and remove them from the source text. Use the AST node attributes `lineno`, `col_offset`, `end_lineno`, and `end_col_offset` (available in Python 3.8+). Ruff’s autofix mechanism does this: it computes a deletion range for each unused name or import statement and applies it to the source.

**Steps:**

1. Read the source file into a string or list of lines.
2. For each unused import (or alias), get the AST node where it’s defined (the `alias` object within `Import`/`ImportFrom`).
3. Determine the slice to delete:
   - If the entire import is unused (no other names in the same statement are used), delete from the start of the import statement to the end of line.
   - If part of a multi-import, delete only the alias text and a trailing or leading comma. This requires identifying the comma location in the line text.
4. Apply deletions in a **single pass** or by sorting deletion spans from bottom to top to avoid recalculating offsets as you delete.
5. Write the modified text back to file.

**Example:**

```python
import ast
from pathlib import Path

code = Path("example.py").read_text().splitlines(keepends=True)
tree = ast.parse("".join(code), filename="example.py")

# Suppose we have `unused_aliases` as a list of tuples (lineno, col_offset, end_lineno, end_col_offset)
# indicating exact spans to delete. These cover import tokens and adjacent commas/spaces.

# Build line_starts to map (lineno, col_offset) to absolute index
line_starts = [0]
for line in code:
    line_starts.append(line_starts[-1] + len(line))

deletions = []
for alias_node in unused_aliases:
    start_line, start_col = alias_node.lineno, alias_node.col_offset
    end_line, end_col = alias_node.end_lineno, alias_node.end_col_offset
    start_index = line_starts[start_line - 1] + start_col
    end_index = line_starts[end_line - 1] + end_col

    # Optionally extend end_index to remove a trailing comma + space
    text_after = "".join(code)[end_index:end_index+2]
    if text_after.startswith(", "):
        end_index += 2

    deletions.append((start_index, end_index))

# Sort and apply deletions
deletions.sort(reverse=True)
full_text = "".join(code)
for start_index, end_index in deletions:
    full_text = full_text[:start_index] + full_text[end_index:]

Path("example_fixed.py").write_text(full_text)
```

**Pros:**

- Preserves comments and formatting outside of removed tokens.
- Minimal textual changes.

**Cons:**

- Managing edge cases (leading/trailing commas, trailing comments) can be tricky.
- Must carefully handle multi-line imports or unusual formatting.

### Handling `__init__.py` Safely

When detecting unused imports in `__init__.py`:

1. **First-Party Imports (within package):**
   - Preferred fix: convert `import module` to a redundant alias `from . import module as module`. This indicates re-export without removing it.
   - Alternatively, if `__all__` exists, add the import name to `__all__` instead of removing.
   - If neither is practical, skip autofix or only warn.

2. **Third-Party/Stdlib Imports:**
   - Removing them might break side effects. Mark such fixes as unsafe or skip automatic removal.

**Example: Safe Re-export in `__init__.py`**

Original:

```python
# __init__.py
import utils  # unused import
```

Safe fix:

```python
# __init__.py
from . import utils as utils  # redundant alias re-export
```

This preserves the import (so that `package.utils` is available) while satisfying the linter by using a redundant alias pattern.

## Putting It All Together: Workflow for Cribo

Below is a step-by-step outline for an AI agent implementing unused import detection and removal in Cribo:

1. **Parse Source**: Use RustPython’s parser (or CPython’s `ast`) to generate an AST for the file.
   ```python
   tree = ast.parse(source_code, filename="module.py")
   ```

2. **Traverse AST (collect bindings/usages)**:
   - Initialize an empty `scope_stack`.
   - Visit nodes in a pre-order traversal (you can use `ast.NodeVisitor` or write a custom walker).
   - On `Import`/`ImportFrom`, for each alias:
     - If module is `"__future__"`, mark as used and continue.
     - If wildcard import (`alias.name == "*"`) – skip adding to import bindings (and optionally record a warning).
     - If `alias.asname == alias.name`, mark as used (re-export), skip further handling.
     - Otherwise, create a binding entry in the current scope: `binding = {"name": alias_name, "type": "import", "used": False, "module": module_name, "node": alias}`.
   - On `Name` nodes in **Load** context, resolve name to the innermost scope where it’s defined:
     - If the binding’s `type` is `"import"`, mark `binding["used"] = True`.
   - On assignments or definitions to a name `N` (in **Store** context or `FunctionDef`/`ClassDef` name):
     - If `N` is an import binding in the same scope and not yet used:
       - If this is a loop variable – record F402.
       - Otherwise – record F811.
       - Mark that import binding as handled (remove it from tracking).
     - Then proceed to record the new local binding for `N`.

3. **Detect `__all__`**:
   - After (or during) traversal, find assignments to `__all__` where the right-hand side is a list or tuple of string literals.
   - Collect those strings into `all_exports`.
   - Mark any import binding whose `name` appears in `all_exports` as `used`.

4. **Handle Allowlist of Side-Effect Imports**:
   - Load a config option `allowed_unused_imports` (list of module prefixes).
   - For each import binding, if `binding["module"]` starts with any prefix in the allowlist, mark `binding["used"] = True`.

5. **Skip `__future__` and Wildcard Imports**:
   - Exclude imports with `module=="__future__"` from unused detection.
   - Exclude wildcard imports (`alias.name=="*"`) from F401.

6. **Report Unused Imports**:
   - For each import binding with `used == False`, emit F401: “`<name>` imported but unused.”

7. **Collect Fix Spans**:
   - For each unused import binding, get `(lineno, col_offset, end_lineno, end_col_offset)` from the AST.
   - If all aliases in an import statement are unused, record the span for the entire line.
   - Otherwise, record spans for each alias, and adjust spans to include adjacent commas/spaces.

8. **Apply Fixes**:
   - **Option A (AST-Unparse)**: Use `RemoveUnusedImports` transformer and `ast.unparse()`.
   - **Option B (Text-Edit)**: Use deletion spans with text slicing.

9. **Special Handling in `__init__.py`**:
   - If filename ends with `/__init__.py`, for each unused import:
     - If it’s a first-party module (relative import or matches package name), transform to redundant alias or add to `__all__`.
     - Else (third-party/stdlib), skip or mark as unsafe.

10. **Write Updated Code** to the output file.

11. **Run Tests**:
    - Unit tests for each case.
    - Integration tests comparing transformed code against expected outputs.
    - Verify code is syntactically valid after fixes.

## Practical Examples

### Example A: Single Unused Import

**Input (`module.py`):**

```python
import os  # line 1
def foo():
    return 42
```

Steps:

1. Parse AST.
2. Record binding `{"name":"os","module":"os","used":False}`.
3. No usage of `os`. Report F401 at line 1.
4. Span covers `(1, 0)-(1, 9)`. Remove entire line. Output:
   ```python
   def foo():
       return 42
   ```

### Example B: Multiple Names, Partial Use

**Input (`module.py`):**

```python
from math import sqrt, pi, sin  # line 1
area = pi * r**2                 # line 2
```

Steps:

1. Parse AST.
2. Bind `sqrt`, `pi`, `sin`. Mark `pi.used=True` when seen in line 2.
3. Report F401 for `sqrt` and `sin`.
4. Spans: remove `"sqrt, "` and `", sin"`. Result:
   ```python
   from math import pi
   area = pi * r**2
   ```

### Example C: Redundant Alias (Re-export)

**Input (`__init__.py`):**

```python
from .helpers import Helper as Helper  # line 1
```

Step:

- `alias.name == alias.asname`, treat as used. No F401.

### Example D: Shadowed Import (F811)

**Input (`module.py`):**

```python
import math  # line 1
math = "approximate"  # line 2
```

Steps:

1. Bind `{"name":"math","used":False}` at line 1.
2. Assignment shadows `math` at line 2. `used=False` → F811 at line 2. Remove import on line 1.
3. Output:
   ```python
   math = "approximate"
   ```

### Example E: Side-Effect Import Allowed

**Input (`module.py`):**

```python
import hvplot.pandas  # line 1, side-effect
```

If `allowed_unused_imports = ["hvplot.pandas"]`, mark as used. No F401.

### Example F: `__future__` Import

**Input (`module.py`):**

```python
from __future__ import annotations  # line 1
```

Skip in unused detection. No F401.

### Example G: Wildcard Import

**Input (`module.py`):**

```python
from config import *  # line 1
DEBUG = True if DEBUG else False
```

Skip in unused detection. Optionally warn about star import (F403).

## Conclusion

By parsing the code into an AST and tracking symbol definitions and uses by scope, Cribo can effectively detect unused imports in the same way Ruff (and Pyflakes) do. Key aspects include treating certain imports as used due to conventions (`__all__`, redundant alias) and identifying scenarios where an import is unused because it’s overshadowed by other code. Once identified, constructing minimal edits (either via AST unparse or careful text slicing) will remove the unused imports while leaving the rest of the code intact.

In summary, to implement F401 in Cribo:

- **Parse** the source.
- **Traverse** AST to collect import bindings and mark uses.
- **Handle** special cases (`__all__`, redundant alias, shadowing, `__future__`, wildcards, allowlist).
- **Report** unused imports (F401).
- **Compute fix spans** and apply minimal edits or use AST unparse.
- **Special-case** `__init__.py` to preserve package interfaces.
- **Test** thoroughly.

Following this guide allows an AI agent or developer to implement unused import detection and autofix in Cribo directly, without referencing Ruff’s codebase.

## References

1. Pyflakes’ unused import logic.
2. Ruff’s handling of F401, F402, F811, F403, etc.
3. Python AST documentation (`ast.Import`, `ast.ImportFrom`, `ast.Name`).
4. Ruff’s autofix mechanism using AST node spans.
5. Handling of re-exports (`__all__` and redundant alias imports).
6. Exemptions for side-effect imports.
7. Special-case rules for `__future__` and wildcard imports.
8. Error codes: F401, F402, F811, F403.
