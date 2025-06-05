# Ruff AST Transformation Patterns - Impact Analysis

## Executive Summary

After analyzing ruff's AST transformation patterns, there are significant architectural differences between ruff's approach and Serpen's current implementation. Ruff uses a more robust, type-safe visitor pattern with separate traits for read-only traversal and mutation, while Serpen uses a more manual approach.

## Key Findings

### 1. Ruff AST Architecture Advantages

**Mature Type Analysis Infrastructure:**

- Dedicated `ruff_python_semantic` crate with comprehensive semantic model
- Built-in TYPE_CHECKING block detection with sophisticated alias tracking
- Advanced import resolution and symbol tracking
- Production-tested typing module awareness

**Superior AST Structure:**

- More ergonomic and idiomatic Rust AST representation
- Better integration with modern Python features
- Comprehensive visitor patterns with transformation support
- Built-in support for string type annotations and forward references

### 2. Current Implementation vs. Ruff Comparison

| Feature                 | rustpython-parser              | ruff AST                                      |
| ----------------------- | ------------------------------ | --------------------------------------------- |
| TYPE_CHECKING Detection | Manual implementation required | Built-in with `is_type_checking_block()`      |
| Import Tracking         | Custom HashMap-based system    | Semantic model with qualified name resolution |
| Alias Resolution        | Basic string matching          | Full semantic analysis with context           |
| typing.cast Handling    | Manual pattern matching        | Semantic model with qualified name matching   |
| Annotation Processing   | Limited AST node types         | Comprehensive annotation framework            |
| Forward References      | Manual string parsing          | Built-in string annotation parsing            |

### 3. Specific Type Stripping Advantages with Ruff AST

#### A. TYPE_CHECKING Block Detection

**Current approach with rustpython-parser:**

```rust
// Manual pattern matching - fragile and incomplete
fn is_type_checking(&self, expr: &Expr) -> bool {
    match expr {
        Expr::Name(name) => name.id == "TYPE_CHECKING",
        Expr::Attribute(attr) => attr.attr == "TYPE_CHECKING",
        _ => false,
    }
}
```

**With ruff AST:**

```rust
// Robust semantic analysis - handles all patterns
use ruff_python_semantic::analyze::typing::is_type_checking_block;

if is_type_checking_block(if_stmt, semantic) {
    // Remove entire block with confidence
}
```

#### B. Import Tracking and Resolution

**Current approach:**

```rust
// Limited import tracking
struct ImportTracker {
    typing_aliases: HashMap<String, String>,
    typing_modules: HashSet<String>,
}
```

**With ruff AST:**

```rust
// Comprehensive semantic model
use ruff_python_semantic::SemanticModel;

// Automatic qualified name resolution
if semantic.match_typing_expr(expr, "TYPE_CHECKING") {
    // Handles: typing.TYPE_CHECKING, from typing import TYPE_CHECKING as TC, etc.
}
```

#### C. typing.cast Rewriting

**Current approach requires manual pattern matching:**

```rust
fn is_typing_cast(&self, call: &ExprCall) -> bool {
    match &call.func {
        Expr::Name(name) => name.id == "cast",
        Expr::Attribute(attr) => attr.attr == "cast",
        _ => false,
    }
}
```

**With ruff AST:**

```rust
// Semantic understanding of qualified names
if semantic.match_typing_expr(&call.func, "cast") {
    // Confidently rewrite typing.cast(T, value) -> value
    return Some(call.args[1].clone());
}
```

#### D. Annotation Processing

**Ruff provides specialized annotation handling:**

```rust
use ruff_linter::rules::flake8_type_checking::helpers::quote_annotation;
use ruff_python_ast::types::annotation::Annotation;

// Built-in support for complex annotation scenarios:
// - String literal type annotations ("List[int]")
// - PEP 585 generics (list[int] vs List[int])
// - PEP 604 union syntax (int | str vs Union[int, str])
// - Annotated types with metadata
```

### 4. Migration Strategy Impact

#### Minimal Migration Approach

Keep rustpython-parser for existing bundling logic, add ruff AST specifically for type stripping:

**Pros:**

- Minimal disruption to existing codebase
- Can leverage ruff's type analysis immediately
- Gradual migration path

**Cons:**

- Dual parser overhead (performance cost)
- Code complexity with two AST systems
- Potential AST conversion overhead

#### Full Migration Approach

Replace rustpython-parser with ruff AST throughout Serpen:

**Pros:**

- Single, more powerful AST system
- Future-proof with active development (ruff is heavily maintained)
- Access to all ruff semantic analysis features
- Better error reporting and diagnostics

**Cons:**

- Significant refactoring required
- Risk of breaking existing functionality
- Learning curve for new AST API

### 5. Pyrefly Evidence

Pyrefly's successful adoption of ruff AST demonstrates:

```toml
# pyrefly/Cargo.toml
ruff_python_ast = { git = "https://github.com/astral-sh/ruff/" }
ruff_python_parser = { git = "https://github.com/astral-sh/ruff/" }
ruff_source_file = { git = "https://github.com/astral-sh/ruff/" }
ruff_text_size = { git = "https://github.com/astral-sh/ruff/" }
```

**Key observations:**

- Pyrefly successfully uses ruff AST for comprehensive Python type analysis
- Production-ready implementation with sophisticated type processing
- Active development and maintenance

### 6. Performance Considerations

**Parser Performance:**

- Ruff's parser is optimized for production use
- Significantly faster than rustpython-parser in benchmarks
- Better memory efficiency for large codebases

**Type Analysis Performance:**

- Ruff's semantic model provides cached analysis
- Qualified name resolution is O(1) vs manual HashMap lookups
- Built-in import resolution eliminates custom tracking overhead

## Recommendations

### Phase 1: Proof of Concept (Recommended)

Implement type stripping using ruff AST alongside existing rustpython-parser:

```rust
// New type stripper module
use ruff_python_ast as ast;
use ruff_python_parser::parse_module;
use ruff_python_semantic::SemanticModel;

pub struct RuffTypeStripper {
    semantic: SemanticModel,
}

impl RuffTypeStripper {
    pub fn strip_types(&mut self, source: &str) -> Result<String> {
        let module = parse_module(source)?;
        let semantic = SemanticModel::new(&module);
        // Use ruff's built-in type checking analysis
        // ...
    }
}
```

**Benefits:**

- Quick evaluation of ruff AST advantages
- Minimal risk to existing functionality
- Ability to benchmark performance differences
- Path to gradual migration

### Phase 2: Full Migration (If Phase 1 Succeeds)

Replace rustpython-parser completely with ruff AST:

```rust
// Update Cargo.toml
[dependencies]
ruff_python_ast = "0.1"
ruff_python_parser = "0.1"
ruff_python_semantic = "0.1"
# Remove: rustpython-parser = "..."
```

## Implementation Complexity Analysis

### With rustpython-parser (Current Path)

**Estimated Implementation Effort: 4-6 weeks**

Required components:

1. Manual TYPE_CHECKING detection (complex, error-prone)
2. Custom import tracking system (medium complexity)
3. String annotation parsing (high complexity)
4. typing.cast pattern matching (medium complexity)
5. Annotation stripping logic (high complexity)

**Risk Factors:**

- High likelihood of edge case bugs
- Difficult to maintain comprehensive type support
- Performance overhead from manual analysis

### With ruff AST (Recommended Path)

**Estimated Implementation Effort: 2-3 weeks**

Leveraged components:

1. `is_type_checking_block()` - built-in ✅
2. `SemanticModel` for import tracking - built-in ✅
3. `match_typing_expr()` for qualified names - built-in ✅
4. String annotation parsing - built-in ✅
5. Annotation visitor patterns - built-in ✅

**Primary implementation tasks:**

1. AST transformation visitor for annotation removal
2. Integration with existing bundler workflow
3. Configuration and testing

## Code Examples

### Type Checking Block Removal with Ruff AST

```rust
use ruff_python_semantic::analyze::typing::is_type_checking_block;

impl TypeStripper {
    fn visit_if_stmt(&mut self, if_stmt: &mut StmtIf) -> Vec<Stmt> {
        if is_type_checking_block(if_stmt, &self.semantic) {
            // Confidently remove TYPE_CHECKING block
            // Handles all patterns: typing.TYPE_CHECKING, TYPE_CHECKING, TC, etc.
            return vec![];
        }
        // Continue with normal processing
        vec![Stmt::If(if_stmt.clone())]
    }
}
```

### Import Cleanup with Semantic Model

```rust
impl TypeStripper {
    fn cleanup_imports(&mut self, module: &mut ModModule) {
        module.body.retain(|stmt| {
            if let Stmt::ImportFrom(import) = stmt {
                if let Some(module_name) = &import.module {
                    // Use semantic model for qualified name resolution
                    let qualified_name = QualifiedName::from_dotted_name(module_name);
                    return !self
                        .semantic
                        .match_typing_qualified_name(&qualified_name, "typing");
                }
            }
            true
        });
    }
}
```

## Conclusion

**Ruff AST provides substantial advantages for type hint stripping implementation:**

1. **Reduced Implementation Time:** 2-3 weeks vs 4-6 weeks
2. **Higher Reliability:** Production-tested semantic analysis vs custom implementation
3. **Future-Proof:** Active development and maintenance
4. **Better Performance:** Optimized parser and semantic model
5. **Comprehensive Coverage:** Handles edge cases automatically

**Recommendation:** Proceed with Phase 1 proof of concept using ruff AST for type stripping, with a clear path to full migration if successful.

The evidence strongly suggests that ruff AST would dramatically simplify the type hint stripping implementation while providing superior reliability and performance compared to the current rustpython-parser approach.
