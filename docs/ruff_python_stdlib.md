# Rust Crates for Python Language Elements

**The ruff ecosystem provides the ideal solution for your needs, with `ruff_python_stdlib` being the top recommendation since you're already using `ruff_python_ast`.**

## Primary recommendation: ruff_python_stdlib

Since you're already using `ruff_python_ast` for AST parsing, **`ruff_python_stdlib`** from the same ecosystem is your best choice. This crate is part of the Ruff project (astral-sh/ruff), the fastest Python linter written in Rust with over 17,400 GitHub stars and extremely active maintenance.

### Key features of ruff_python_stdlib

The crate provides exactly what you need for name conflict resolution:

- **Comprehensive built-in function lists** (print, len, str, etc.) with version-specific support
- **Complete Python keyword sets** (def, class, if, else, etc.) across different Python versions
- **Full standard library module listings** with metadata about mutability and types
- **Multi-version support** for Python 3.8 through 3.13
- **Optimized performance** designed specifically for fast Python tooling

The API design makes integration straightforward. Lists are exposed as static arrays and const functions, allowing efficient lookups during AST traversal. Since it's from the same ecosystem as `ruff_python_ast`, you'll have perfect compatibility and consistent design patterns.

### Integration approach

Here's how you would typically use it:

```rust
use ruff_python_stdlib::builtins;

// Check if an identifier conflicts with Python built-ins
fn has_builtin_conflict(name: &str, python_version: PythonVersion) -> bool {
    builtins::is_builtin(name, python_version)
}
```

The crate handles version-specific differences automatically, so you can specify which Python version you're targeting and get the appropriate lists.

## Alternative option: rustpython-parser

If you need a different approach, **`rustpython-parser`** from the RustPython project (17,400+ stars) is another solid choice. While it's primarily a parser, it includes comprehensive Python language element definitions:

- Complete keyword recognition built into the lexer
- AST node definitions that match Python's official AST
- Support for the latest Python 3.13+ syntax
- Well-documented with extensive examples

However, since you're already in the ruff ecosystem, switching to rustpython-parser would mean mixing two different AST representations, which could complicate your codebase.

## Why avoid maintaining custom lists

Your instinct to use existing crates instead of maintaining custom PYTHON_BUILTINS, PYTHON_KEYWORDS, and STD_LIB_MODULES lists is spot-on. These crates offer several advantages:

1. **Automatic updates** when new Python versions add built-ins or stdlib modules
2. **Community-verified accuracy** from projects used in production by thousands
3. **Version-aware APIs** that handle differences between Python releases
4. **Performance optimizations** specifically for AST tooling use cases
5. **Reduced maintenance burden** - no need to track Python language changes

## Production readiness indicators

Both recommended crates show strong signs of reliability:

- **Active maintenance**: Daily commits and regular releases
- **Production usage**: Ruff is used by major projects like Django, FastAPI, and Pandas
- **Comprehensive testing**: Both projects have extensive test suites
- **Strong communities**: Active issue tracking and quick bug fixes
- **Performance focus**: Designed for speed in AST processing contexts

## Implementation recommendation

For your specific use case of name conflict resolution and AST rewriting with `ruff_python_ast`, I strongly recommend adopting `ruff_python_stdlib`. It provides a production-ready, well-maintained solution that integrates seamlessly with your existing toolchain. The crate is actively maintained as part of one of Rust's most successful Python tooling projects, ensuring long-term support and regular updates as Python evolves.

The migration from custom lists should be straightforward - the ruff crates are designed with clean APIs that make it easy to check for conflicts and retrieve comprehensive lists of Python language elements for any supported Python version.
