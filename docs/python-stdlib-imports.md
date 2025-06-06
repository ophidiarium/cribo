# Python Standard Library Import Behavior and Bundling Considerations

## 1. Side Effects of Standard Library Imports

In general, Python's standard library modules strive to minimize side effects on import, but they are not *guaranteed* to be side-effect free. Importing a module executes all top-level code in that module, which can perform arbitrary actions. Most standard modules only initialize data or definitions, but a few do have notable side effects. For example, **importing** the `tkinter` GUI module is known to **spawn a new thread** on macOS systems. Another classic example is the whimsical `antigravity` module – merely doing `import antigravity` will **launch a web browser** to load an XKCD comic. These are intentional (if surprising) behaviors. More broadly, an import can do things like open files, set environment variables, register plugins, or start background tasks.

The Python issue tracker notes that *"Many stdlib modules have side effects on import. For example, `import antigravity` opens a web browser. An import can open files, spawn threads, run programs, etc."* In practice, such aggressive side effects are rare in the standard library, but **not nonexistent** – even seemingly innocuous modules may perform some initialization (e.g. seeding random number generators, reading locale settings, or registering codecs) as a side effect of import. The important takeaway is that one cannot assume *all* standard library imports are free of side effects; module imports execute code, and any such code *could* have effects beyond simple definitions. The vast majority of standard modules keep import-time work to a minimum, but **there are known exceptions**, so one should be mindful especially when bundling or reordering imports.

## 2. Performance Impact of Hoisting Imports vs. Lazy Imports

**When** and **where** you import modules can affect application startup time and runtime performance. Hoisting all imports to the top of a file (the usual style) means every import is executed at startup, potentially loading many modules even if some are never actually used in that run. This can slow down startup or increase memory usage unnecessarily. In contrast, delaying certain imports – e.g. placing them inside functions or conditional blocks – defers the cost until (and unless) the functionality is needed. This trade-off is well recognized in the Python community. One experienced Pythonista notes that *"an import statement is not free. Importing modules in a hot path may add some (small) overhead."* By keeping infrequently-used imports inside functions, you avoid paying the initialization cost unless the function is called. This can improve responsiveness for programs that only sometimes need a heavy module. For instance, if you have an **expensive dependency** that is only used for a rare operation, you might **delay importing** it until necessary – thereby reducing the baseline startup time.

On the other hand, hoisting everything at the top ensures that the first call to any function doesn't incur an import delay. If a function is called repeatedly in a tight loop, doing a redundant `import` each time (even though Python will fetch it from cache after the first time) adds a slight overhead for the name lookup and `sys.modules` check on every call. In practice, the runtime overhead of a repeated import lookup is usually negligible (after the first import, the module is cached, so it's essentially a dictionary lookup). However, the *initial* import of a large module can be significant – especially if it involves I/O or complex initialization.

From a performance perspective, **hoisting imports** incurs all the cost up front, which can harm startup time if many modules are loaded unconditionally. **Lazy imports** spread or avoid that cost. Recent discussions (and even a proposed PEP for lazy imports) have quantified this: avoiding importing unused modules at startup can **save dozens of milliseconds** for applications that would otherwise load dozens of modules needlessly. This is why there's interest in automatic lazy-import mechanisms. The flip side is that the first use of a lazily-imported feature might incur a noticeable one-time lag. In summary, hoisting imports makes the *first run* of any function faster (since all modules are preloaded) but potentially slows program startup, whereas lazy imports can significantly improve startup time or memory footprint by only loading what's actually needed. In a bundled single-file scenario, one should balance these concerns: it may be wise to keep imports inside functions for large modules that might not be used every run, but avoid putting very cheap or always-used imports in inner scopes. (Additionally, maintainability and clarity matter – too many hidden imports can complicate debugging, as ImportErrors will occur later in execution rather than on startup.)

## 3. The "Double Import Trap" and Standard Library Usage

The "double import trap" refers to a situation where the same module gets *imported twice* under different names, leading to two distinct copies of its state in memory. This typically happens due to Python's import system treating module names as keys in `sys.modules`. A classic scenario is when a module is accessible via two paths – for example, as a top-level module and as part of a package – causing Python to load it separately each way. In general application development, this can occur if you manipulate `sys.path` incorrectly or use package and module names that shadow each other. A historical example was Django <=1.3, where an app could be imported both as `app` and as `site.app`, yielding two copies of the module loaded (each with its own state). This is, of course, undesirable and was fixed in later versions of Django. In modern Python, the **double import trap is mostly an issue with user code** (especially when running modules as scripts). The standard library itself is designed so that each module has one canonical import name, so you *would not expect* to import the same stdlib module twice under different names under normal conditions. In other words, if you do `import math` in multiple places, Python will only load the `math` module once (the first time) and then reuse it from the module cache on subsequent imports. That cache (discussed more below) prevents truly loading a module twice.

However, there are a few ways the double import trap *could* manifest even with standard library modules, usually due to user error or unusual execution environments. One common pitfall is running a module as a script. When you execute a module directly (for instance, running `python module.py`), that file is loaded under the name `__main__`. If that script internally does `import module` (assuming the same module is reachable via PYTHONPATH), Python will **import it a second time** under its real name, since `__main__` is a distinct namespace. This means all the module-level code runs again, and you end up with duplicate copies of objects – a textbook double-import scenario. This can happen with any module (standard library or not) if you invoke it incorrectly. The standard advice is to use the `-m` switch (e.g. `python -m module`) to run a module to avoid this. Another scenario is name shadowing: if you name your own script or package the same as a standard library module (say `logging.py` or `random.py` in your working directory) and then do `import logging` or `import random`, you might end up importing your own file as one module and the real library as another, or otherwise confuse the import system. In such cases, you might see odd behavior because you effectively have two modules with the same or related code.

In summary, **in typical use the standard library doesn't suffer from double imports** – Python will load each stdlib module once. But the trap can arise in edge cases (running modules as scripts, naming collisions, or bundling mishaps). If you are bundling code into one file, be careful that you don't introduce alias names for modules that could cause Python to think it's importing something new. Ensuring a one-to-one correspondence between logical module names and code is key. And thanks to `sys.modules`, Python will not re-execute a module import as long as the exact module name has already been imported (avoiding duplicate side effects or initialization).

## 4. `typing.TYPE_CHECKING` at Runtime and Aliasing Concerns

The constant `typing.TYPE_CHECKING` is a special flag introduced for the benefit of static type checkers (e.g. Mypy, Pyright). **At runtime, `TYPE_CHECKING` is always `False`**, but type checkers treat it as `True` during type analysis. This allows you to guard imports or code that are only needed when type-checking (like importing modules solely for type annotations) inside an `if TYPE_CHECKING:` block. At **runtime** the condition is false, so the block is skipped (avoiding import overhead or runtime circular import issues), but the type checker sees the block as active and can resolve the names for type checking purposes.

Because `TYPE_CHECKING` is just a normal Python boolean (set to `False` in the `typing` module), you might wonder if you can alias it or define it yourself. For example, doing `from typing import TYPE_CHECKING as tc` or even `tc = typing.TYPE_CHECKING` will give you a variable `tc` that is also `False` at runtime. **Technically, this is "valid" Python code and will work at runtime** – your alias `tc` will be False, and you could write `if not tc: ...` or similar. However, it's **not safe if you rely on static type checkers** understanding your intent. Type checkers have special logic for the name `typing.TYPE_CHECKING` (and historically even any global named `TYPE_CHECKING`): they know to assume it is True. If you alias it to a different name, the static analyzer likely won't recognize it as the magic constant. In practice, MyPy and other checkers will **not treat an alias like `tc` as True during type checking**, so code under `if tc:` might be skipped or incorrectly analyzed. In that example, using `if TYPE_CHECKING:` produced *no* error (the type checker ignored the always-false branch), but using `if amogus:` (an alias) caused an error because the type checker didn't realize this was the same constant and actually checked that branch. The takeaway is that you should **not rename or alias `TYPE_CHECKING`** if you want the intended behavior. Keep using it directly from `typing`.

In summary, at runtime `TYPE_CHECKING` is just a False constant (so guarding code with it prevents execution of that code), and it's safe in that sense. Just remember that its **power lies in static analysis**, and aliasing it will confuse the static analysis while offering no real benefit at runtime.

## 5. Python's Import Mechanics, Module Caching, and Code Bundling

Python's import system has some important runtime characteristics that affect how you bundle code into a single file. First, Python employs a **module cache** (`sys.modules`) to ensure that each module is loaded only once per interpreter session. When you import a module, Python will check `sys.modules` to see if that module's name is already present; if so, it returns the existing module object and **skips re-initializing it**. This caching is crucial for performance and for preventing the double-import issues discussed above. It means that if your single-file bundle somehow triggers an import of a standard library module (or any module) that was already imported, Python won't reload it anew – you'll get the cached version, and thus the module-level code (with any side effects) runs only once.

When it comes to **bundling first-party code into one file**, there are a couple of approaches, but all revolve around leveraging or simulating Python's import machinery. If you literally concatenate all your module source files into one big `.py` file, you no longer have distinct module namespaces – you'd effectively create one giant module. In such a scenario, you must remove or adjust internal imports, because `import mymodule` (where `mymodule` was originally a separate file) might either fail or import a different module if left unchanged. A naive concatenation can lead to name conflicts (e.g., two modules both define a global `DATA` or `config` variable, now in one file) and loses the benefits of module isolation. A better strategy that some bundlers use is to **embed modules and hook the import system**. For example, Python's own **zip importer** mechanism (PEP 273/PEP 302) allows multiple files to be packaged in a zip archive and still be imported as individual modules – the zip file acts like a combined container, but the import system treats each file as its own module, caching each in `sys.modules` normally. In the single-file scenario (not even a zip), a similar trick is to inject a custom importer. One could store the source code of submodules as strings inside the big file and install a `meta_path` finder/loader that knows how to retrieve those strings as module code when that module is imported. In other words, you **simulate a filesystem within your one file**. This technique was demonstrated by developers who needed to ship a single Python file but maintain multiple modules inside it (such as for Kaggle competitions or plugins). The custom importer approach preserves the module boundaries – each embedded module executes in its own namespace (module dict) and gets an entry in `sys.modules` under its name, so it behaves just like a normal multi-file package.

If you don't go the import-hook route and instead choose to merge code, then you'll have to be very cautious about how imports are handled. One approach is to **manually populate `sys.modules`** for your submodules to fool the import system. For instance, your combined script might execute the contents of what used to be module `X` and assign it to a new `module` object for `X` in `sys.modules`. Then when other parts do `import X`, Python finds it already in the cache and uses it. Tools like PyInstaller and others do similar things (though PyInstaller actually bundles bytecode in an archive and uses a bootloader, which is more complex but conceptually related – it preloads modules into `sys.modules`). The key points regarding runtime import mechanics are: **(a)** each module is only executed once (on first import) and thereafter cached; **(b)** module code is executed top-to-bottom at import time (so ordering of imports can matter for side effects or dependencies); **(c)** the import system can be extended to load code from non-traditional sources (like zipped packages or strings in memory). When bundling, you leverage these by either preloading everything (execution at startup) or by providing a custom loader that knows how to fetch module code from the single-file bundle on demand.

In summary, from a runtime perspective, bundling doesn't fundamentally change how Python imports modules – it just means you have to package your code in a way that Python's import system can still find the "modules." Whether that's by merging into one namespace or by cleverly intercepting import requests, the behaviors of `sys.modules` and import caching still apply. Once a module (standard library or your own) is loaded, it will sit in `sys.modules` and further imports of that name will reuse the cached module.

## 6. Special Case: `from __future__` Imports

- **Purpose**\
  A `from __future__ import feature_name` statement tells the Python compiler to enable a feature that will become standard in a future release. In other words, it lets you use new syntax or semantics **before** it is the default. For example, in Python 3.9 you can write:
  ```python
  from __future__ import annotations
  ```

  so that all function annotations are stored as strings rather than evaluated at definition time (PEP 563).

- **Special-casing in the compiler**\
  The parser/compiler treats any `from __future__ import ...` statement specially. When the compiler sees it, it sets an internal "feature flag" so that it will parse subsequent code under the new rules (e.g., expect new syntax or delay annotation evaluation). Those flags affect how code is compiled to bytecode, not how it runs at runtime.

- **Runtime behavior**\
  Under the hood, `__future__` is a real module (you can do `import __future__` at runtime); it lives in Lib/**future**.py. However, the compiler looks for these import statements before actually executing module code. If you place

  ```python
  from __future__ import something
  ```

  anywhere other than the top of the file (right after the module docstring and before any other code or imports), the compiler will raise a SyntaxError. In other words, even though `__future__` is a normal module, its imports must appear first so that the compiler can see them and set the appropriate compile-time flags before any other code is parsed and turned into bytecode.

  Quote from the documentation:
  "Imports of the form from `__future__` import feature are called future statements. These are special-cased by the Python compiler to allow the use of new Python features in modules containing the future statement before the release in which the feature becomes standard… While these future statements are given additional special meaning by the Python compiler, they are still executed like any other import statement and the `__future__` exists and is handled by the import system the same way any other Python module would be."

- **Key points to remember**
  1. **Placement matters:**
     - Must be at the top of the module (after any docstring, but before any other code or imports).
     - If you put it later, you get a SyntaxError.
  2. **Only compile-time effect:**
     - The act of importing `__future__.feature` does not itself "run" new code at runtime; it merely flips a compile-time switch so that the compiler produces different bytecode (for instance, treating annotations as strings).
  3. **Real module exists:**
     - At runtime, `__future__` is just another module you could inspect (e.g., `help(__future__)`), but by then the compiler has already acted on that statement.

Official reference (Python 3.9):
https://docs.python.org/3.9/library/future.html

### 6.1 How CPython Enforces Future Imports at Compile Time

1. **First pass (parsing)**\
   When you run `python mymodule.py`, the interpreter first tokenizes and parses your source file. During parsing, it looks for any `from __future__ import ...` lines. If the compiler sees one, it records which feature(s) to enable (for that translation unit).

2. **Syntax checking under new rules**\
   After setting the feature flags, the parser applies the grammar changes associated with that feature. For example, if you did:

   ```python
   from __future__ import annotations
   def f(x: MyType) -> None: ...
   ```

   then the parser does not immediately resolve MyType at compile time—because under PEP 563 all annotations are postponed (i.e., stored as strings).

3. **Bytecode generation**\
   Once parsing is finished (with the future flags in place), CPython generates .pyc bytecode. The resulting bytecode includes a marker in the code object's co_flags (internally called CO_FUTURE_<feature>) so that at runtime the VM knows which features to honor (e.g., skip evaluating annotations).

4. **Runtime**\
   By the time you actually start executing the module, the compilation has already happened with those "future" flags baked in. Importing `__future__` at runtime is a no-op except for setting the compiled-in flags (which have already taken effect).

In short, "future" imports are compiler directives masquerading as imports. The import line must be present at parse time so that the compiler can modify its behavior. After that point, runtime simply sees a normal module import in sys.modules, but by then it's too late to change the parse rules.

## 7. Other Special or "Magic" Import Cases

Aside from `from __future__ import ...`, there aren't any other import statements that influence Python's compilation in the same way. All other imports behave purely at runtime:

- **Typical imports** (`import foo` or `from foo import bar`) simply trigger module lookup, loading, and execution of top-level code if it hasn't been loaded already. They do not change how the parser/compiler works.
- **Encoding declarations** (e.g. `# -*- coding: utf-8 -*-`) are not imports but are recognized in the first two lines of a file to tell the lexer what source encoding to use. That is a separate "magic" comment, not an import.

To be explicit:

1. **`__future__` is unique (compiler-level).**
   - It must come first (after docstring).
   - It flips compile-time feature flags.
   - No other standard import has that effect.

2. **No other "from something import ..." is treated specially by the compiler.**
   - For example, `import __main__`, `import __init__`, `import __annotations__`, etc., are all normal modules or attributes. They do not trigger compiler changes.
   - If you wrote `from __future__ import something_else` that isn't a recognized future feature, you will get a compile-time error:

   ```
   SyntaxError: future feature name 'something_else' is not defined
   ```

3. **Python-internal modules with double-underscore names** (e.g. `__main__`, `__init__`, `__pycache__`, etc.) are not special import directives. They follow normal import semantics.

4. **PEP 402 "Executable Zip Archives" / PEP 441**
   - These allow you to bundle many modules into a single .zip file and still have Python import them normally, but they introduce no new "magic import" syntax; they simply extend the machinery at the import-finder level. Nothing in user code (except sys.path or zipimport) changes compile-time behavior.

5. **Namespace packages (PEP 420)**
   - Again, not special import directives. They just let multiple directories on sys.path share a "dotted" package name.

In short, `__future__` is the one and only standard-library import that the CPython compiler treats as a compile-time flag instead of a purely runtime mechanism. No other import has the same "must appear at the top so the compiler sees it first" requirement.

---

End of research report.
