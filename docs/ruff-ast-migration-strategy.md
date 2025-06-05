# Ruff AST Migration Strategy for Serpen

## Executive Summary

This document provides a comprehensive analysis and migration strategy for transitioning Serpen from `rustpython-parser` to ruff's AST modules (`ruff_python_ast`, `ruff_python_parser`). **Migration is NOT RECOMMENDED** based on maintenance quality assessment, API stability concerns, and strategic risk analysis.

## Maintenance Quality Assessment

### rustpython-parser Repository Health

**Summary**: rustpython-parser shows **healthy, active maintenance** with consistent development activity and strong community engagement.

#### Activity Metrics

- **Repository**: 20,118 stars, 1,314 forks, 430 open issues
- **Commit frequency**: Multiple commits per week
- **Release cadence**: Weekly automated releases (`YYYY-MM-DD-main-NN` pattern)
- **Security awareness**: Regular dependency updates and platform compatibility work

#### Maintenance Quality Indicators

- ✅ **Active development**: Multiple commits per week
- ✅ **Consistent releases**: Automated weekly release cycle
- ✅ **Responsive maintenance**: Recent dependency updates, platform fixes
- ✅ **Community health**: Multiple active contributors, external PRs
- ✅ **Cross-platform support**: Active Windows, macOS compatibility work

### ruff Repository Health

**Summary**: ruff shows **exceptional maintenance activity** and enterprise-level development practices with massive scale and resources.

#### Activity Metrics

- **Repository**: 39,692 stars, 1,379 forks, 1,452 open issues
- **Commit frequency**: Multiple commits per day from diverse contributor base
- **Release cadence**: Professional semantic versioning with bi-weekly intervals
- **Enterprise team**: Astral-sh organization with full-time developers

#### Maintenance Quality Indicators

- ✅ **Hyperactive development**: Multiple commits daily across team
- ✅ **Professional releases**: Bi-weekly semantic versioning with comprehensive testing
- ✅ **Enterprise resources**: Full-time team, dedicated infrastructure
- ✅ **Massive community**: 97% larger GitHub community than rustpython-parser
- ✅ **Advanced features**: Cutting-edge Python tooling development

### Comparative Assessment

| Metric               | rustpython-parser | ruff                           | Advantage         |
| -------------------- | ----------------- | ------------------------------ | ----------------- |
| **GitHub Stars**     | 20,118            | 39,692                         | ruff (97% larger) |
| **Commit Frequency** | Weekly commits    | Multiple daily                 | ruff              |
| **Release Cadence**  | Weekly automated  | Bi-weekly semantic             | Both excellent    |
| **Team Structure**   | Community-driven  | Enterprise team                | ruff              |
| **API Stability**    | Public, stable    | Internal (`version = "0.0.0"`) | rustpython-parser |

**Key Insight**: **Both projects are exceptionally well-maintained** - maintenance quality is NOT a differentiator.

## Current State Analysis

### rustpython-parser Usage in Serpen

Serpen uses rustpython-parser extensively across 6 major files:

- **ast_rewriter.rs** (900+ lines) - Core AST transformation logic
- **emit.rs** (650+ lines) - Bundle generation and AST creation
- **bundler.rs** (400+ lines) - Import extraction and module processing
- **unused_imports_simple.rs** (500+ lines) - AST traversal for unused import detection
- **unused_import_trimmer.rs** (300+ lines) - AST filtering and transformation
- **unparser crate** (2500+ lines) - Complete Python code generation from AST

### Current Dependencies

```toml
rustpython-ast = "0.4"
rustpython-literal = "0.4"
rustpython-parser = "0.4"
```

## ruff AST Modules Analysis

### Available Crates

- **`ruff_python_ast`** - Core AST node definitions with enhanced features
- **`ruff_python_parser`** - Parser built on rustpython but with ruff's AST
- **`ruff_python_codegen`** - Code generation capabilities
- **`ruff_python_trivia`** - Comment and whitespace handling
- **`ruff_text_size`** - Advanced source location tracking

### Key Differences

1. **Enhanced AST nodes** with better metadata and type safety
2. **Different parsing API** - no Mode enum, different return types
3. **Improved error handling** with detailed ParseError types
4. **Better source mapping** with ruff_text_size::TextRange
5. **Advanced visitor patterns** including source-order traversal

## Migration Challenges Assessment

### Critical Challenges

#### 1. Complete API Rewrite Required

**Affected**: All 6 major files

- Every `use rustpython_parser::*` statement needs replacement
- All AST node field access patterns require updates
- Pattern matching on `Stmt::*` and `Expr::*` variants needs verification

#### 2. API Stability Risk

**Critical concern**: ruff AST modules marked as internal (`version = "0.0.0"`)

- No public guarantees for AST interface stability
- Rapid development may include breaking changes
- Ties Serpen to ruff's internal APIs and release cycle

#### 3. Unparser Compatibility

**Affected**: unparser crate (2500+ lines)

- Risk of losing comment preservation, exact formatting, or semantic fidelity
- May require building custom unparser for ruff AST
- Integration complexity with full AST migration requirement

### Major Changes Required

#### AST Construction Patterns

```rust
// Current (rustpython-parser)
ast::StmtImport {
    names: vec![ast::Alias {
        name: "types".into(),  // .into() conversion
        asname: None,
        range: Default::default(),  // rustpython range type
    }],
    range: Default::default(),
}

// ruff (requires changes)
ast::StmtImport {
    names: vec![ast::Alias {
        name: "types",  // Direct assignment, different field type
        asname: None,
        range: TextRange::default(),  // Different range type
    }],
    range: TextRange::default(),
}
```

#### Error Handling Changes

```rust
// Current parsing pattern
let parsed = parse(&source, Mode::Module, "module")?;
let Mod::Module(ast) = parsed else {
    return Err(anyhow::anyhow!("Expected module"));
};

// ruff equivalent
let parsed = parse_module(&source)?;  // Different return type
let ast = parsed.into_syntax();       // Different unwrapping
```

## Simplification Potential Analysis

### Code Reduction Opportunities

**1. Complete Unparser Replacement**

- **Eliminate 1,680+ lines**: Replace entire unparser crate with ruff_python_codegen
- **Better reliability**: Production-tested code generator vs. custom implementation
- **Enhanced features**: Superior edge case handling, style preservation, performance

**2. AST Rewriter Modernization**

- **40-45% reduction**: ~871 lines → ~400-500 lines using ruff's visitor patterns
- **Eliminate manual recursion**: Replace recursive AST traversal patterns
- **Advanced transformers**: Leverage ruff's sophisticated transformation framework

**3. Import Analysis Enhancement**

- **unused_import_trimmer.rs**: 30-35% reduction (~508 → ~300-350 lines)
- **unused_imports_simple.rs**: 25-35% reduction (~923 → ~600-700 lines)
- **Better import utilities**: Use ruff's mature import analysis capabilities

**4. emit.rs AST Construction Simplification**

- **Manual AST node creation**: ~400+ lines of complex AST construction methods
- **With ruff_python_codegen**: Simple string templates + parsing
- **Complex nested attribute construction**: Replace manual AST building with simple generation

**Total Potential Code Reduction**: ~2,500+ lines eliminated or simplified

### ruff_python_codegen Capabilities

**Technical advantages:**

- ✅ **Superior fidelity**: Production-proven with comprehensive round-trip tests
- ✅ **Comprehensive Python support**: All Python 3.8+ features including latest syntax
- ✅ **Style preservation**: Automatic indentation, quote, and line ending detection
- ✅ **Performance advantage**: Single-pass generation, optimized for high throughput
- ✅ **Active maintenance**: Part of ruff's actively developed ecosystem

**Integration requirements:**

- ❌ **Full AST migration required**: Cannot use ruff_python_codegen without migrating to ruff AST
- ❌ **Dependency overhead**: Adds ~20 ruff ecosystem crates
- ❌ **API coupling**: Ties Serpen to ruff's internal API stability

## Strategic Risk Analysis

### Risk-Benefit Matrix

| Risk Factor                 | rustpython-parser | ruff           |
| --------------------------- | ----------------- | -------------- |
| **Abandonment Risk**        | ❌ Very Low       | ❌ Very Low    |
| **API Stability Risk**      | ✅ Very Low       | ❌ High        |
| **Feature Stagnation Risk** | ✅ Low            | ❌ Very Low    |
| **Integration Complexity**  | ✅ Low            | ❌ High        |
| **Migration Effort**        | ✅ Zero           | ❌ Significant |

### Decision Framework

With **maintenance concerns eliminated** for both options, the decision reduces to **strategic and technical factors**:

**Primary Decision Factors:**

1. **API Stability** → rustpython-parser advantage (public, stable)
2. **Migration Risk** → rustpython-parser advantage (zero risk)
3. **Development Effort** → rustpython-parser advantage (zero effort)
4. **Technical Benefits** → ruff advantage (superior capabilities)
5. **Future Capabilities** → ruff advantage (ecosystem alignment)

## Strategic Recommendations

### Option A: Continue with rustpython-parser (STRONGLY RECOMMENDED)

- **Rationale**: Both projects are equally well-maintained, but rustpython-parser has stable APIs
- **Benefits**: Zero risk, zero effort, proven stability, sufficient capabilities
- **When to choose**: Default choice given excellent maintenance of both options

### Option B: Strategic Monitoring (RECOMMENDED PARALLEL ACTIVITY)

- **Track ruff API stability**: Watch for public API announcements (`version > "0.0.0"`)
- **Performance baseline**: Benchmark current implementation against requirements
- **Prototype experiments**: Small tests to validate migration assumptions
- **Ecosystem evolution**: Monitor Python tooling ecosystem trends

### Option C: Conditional Migration (FUTURE CONSIDERATION)

- **Trigger condition 1**: ruff publishes stable public APIs (`version >= "1.0.0"`)
- **Trigger condition 2**: Specific technical requirements emerge (performance, features)
- **Trigger condition 3**: Development capacity available for thorough migration

## Migration Implementation Guide (If Proceeding)

### Dependencies and Setup

- [ ] Update workspace Cargo.toml dependencies
  ```toml
  # Remove
  rustpython-ast = "0.4"
  rustpython-literal = "0.4"
  rustpython-parser = "0.4"

  # Add
  ruff_python_ast = "0.8.0"
  ruff_python_parser = "0.8.0"
  ruff_python_codegen = "0.8.0"
  ruff_text_size = "0.8.0"
  ```

### Core File Migrations

#### bundler.rs Migration

- [ ] Replace `use rustpython_parser::ast` with `use ruff_python_ast`
- [ ] Update parsing calls: `parse()` → `parse_module()`
- [ ] Update import statement processing patterns
- [ ] Update module name extraction logic
- [ ] Test dependency resolution with new AST

#### emit.rs Migration

- [ ] Replace all rustpython imports with ruff equivalents
- [ ] Update parsing error handling patterns
- [ ] Rewrite AST node creation methods
- [ ] Update all range handling to use `TextRange`
- [ ] Test bundle generation output

#### AST Rewriter Migration

- [ ] Replace rustpython imports with ruff equivalents
- [ ] Research ruff's transformer/visitor patterns
- [ ] Rewrite `Transformer` trait implementation
- [ ] Update all AST manipulation patterns
- [ ] Test complex import scenarios and name conflicts

#### Unparser Crate Migration

**Option A: Use ruff_python_codegen directly (RECOMMENDED if migrating)**

- [ ] Replace unparser.rs entirely with ruff_python_codegen usage
- [ ] Implement new Generator integration patterns
- [ ] Leverage Stylist for style preservation

**Option B: Build custom unparser for ruff AST (NOT RECOMMENDED)**

- [ ] Rewrite unparser.rs for ruff AST nodes (2500+ lines)
- [ ] Port transformer.rs to ruff AST
- [ ] Maintain all existing unparsing features

### Testing and Validation

- [ ] Create test compatibility framework
- [ ] Update all snapshot tests (30+ files)
- [ ] Run full integration test suite
- [ ] Test with real-world codebases
- [ ] Performance benchmarking vs. current implementation

## Critical Migration Decision Factors

### Meta's Pyrefly: Production Validation of Ruff AST

**Key Finding**: Meta's pyrefly project **extensively uses ruff crates** for its AST infrastructure, providing significant validation of ruff's production readiness.

#### Pyrefly's Ruff Integration

- **Core dependencies**: Uses `ruff_python_ast`, `ruff_python_parser`, `ruff_source_file`, `ruff_text_size`
- **Production scale**: 295+ import statements using ruff modules across the codebase
- **Strategic commitment**: No rustpython-parser dependencies - full commitment to ruff AST
- **Enterprise validation**: Meta's choice demonstrates ruff AST's suitability for large-scale Python tooling

**Strategic Implication**: Pyrefly proves that ruff's internal APIs, while marked `version = "0.0.0"`, are stable enough for production use by major tech companies.

### Type Hints Stripping Task Analysis

**Critical Finding**: ruff AST would **dramatically simplify** the planned type hints stripping implementation.

#### Implementation Complexity Comparison

**With rustpython-parser (current plan):**

- Manual TYPE_CHECKING block detection using custom logic
- Custom import tracking with HashMap-based state management
- Complex qualified name resolution for `typing.cast` handling
- Manual annotation parsing and semantic analysis
- High risk of edge case bugs in complex scenarios

**With ruff AST (potential approach):**

- **Built-in TYPE_CHECKING detection**: `is_type_checking_block()` handles all patterns automatically
- **Semantic model**: Qualified name resolution and import tracking built-in
- **Production-tested**: Battle-tested type handling code from ruff's linter rules
- **Performance benefits**: Faster parsing and better memory efficiency
- **Lower implementation risk**: Leverage extensively tested infrastructure

#### Specific Technical Advantages

1. **TYPE_CHECKING Block Handling**
   - ruff's `flake8_type_checking` module provides sophisticated detection
   - Handles complex patterns: nested blocks, aliased imports, qualified names
   - Production-tested across thousands of Python codebases

2. **Import Analysis**
   - `SemanticModel` eliminates need for custom import tracking
   - Built-in qualified name resolution for `typing.cast` rewriting
   - Automatic handling of import aliases and from-imports

3. **Annotation Processing**
   - Comprehensive support for all annotation types (function, variable, class)
   - Built-in string annotation parsing for forward references
   - Handles `from __future__ import annotations` automatically

**Implementation Time Estimate:**

- rustpython-parser approach: Complex manual implementation
- ruff AST approach: Leverage existing infrastructure with minimal custom code

## Final Conclusion

The analysis reveals **two critical new factors** that fundamentally change the migration decision:

### Key Findings

1. **✅ Both dependencies are exceptionally healthy**: Maintenance quality is not a differentiator
2. **✅ rustpython-parser exceeded expectations**: Weekly releases, active team, security awareness
3. **✅ ruff shows enterprise-level maintenance**: Professional processes, massive community
4. **✅ Meta validates ruff AST in production**: Pyrefly demonstrates enterprise-scale viability
5. **✅ Type stripping dramatically simplified**: Built-in infrastructure vs complex manual implementation
6. **❌ API stability remains critical**: ruff's internal APIs still present integration risk

### Updated Final Recommendation: **STRATEGIC RECONSIDERATION**

The **two critical new factors** fundamentally alter the migration calculus:

#### Factor 1: Enterprise Production Validation

Meta's pyrefly demonstrates that ruff's internal APIs are **production-stable** despite `version = "0.0.0"` marking. This significantly reduces API stability concerns.

#### Factor 2: Type Stripping Implementation Advantage

ruff AST would **dramatically simplify** the planned type hints stripping feature, leveraging built-in infrastructure instead of complex manual implementation.

### Updated Decision Matrix

| Factor                        | Weight | rustpython-parser      | ruff Migration             | Winner            |
| ----------------------------- | ------ | ---------------------- | -------------------------- | ----------------- |
| **Risk Level**                | HIGH   | ✅ Very Low            | ⚠️ Medium                   | rustpython-parser |
| **Development Cost**          | HIGH   | ✅ Zero                | ❌ Significant             | rustpython-parser |
| **API Stability**             | HIGH   | ✅ Stable              | ⚠️ Medium (Meta validation) | rustpython-parser |
| **Type Stripping Complexity** | HIGH   | ❌ Complex manual impl | ✅ Built-in infrastructure | **ruff**          |
| **Production Validation**     | MEDIUM | ✅ Ecosystem usage     | ✅ Meta/pyrefly            | **TIE**           |
| **Code Quality Potential**    | MEDIUM | ✅ Adequate            | ✅ Superior                | ruff              |

**Updated Assessment**: **More balanced** - ruff gains significant advantages while risks are reduced.

### Revised Strategic Options

**Option A: Hybrid Approach for Type Stripping (NEW RECOMMENDATION)**

- **Implement type stripping using ruff AST** as a targeted migration
- Keep existing bundling logic on rustpython-parser
- Validate ruff AST integration with limited scope before broader migration
- **Benefits**: Dramatic simplification of type stripping + risk mitigation

**Option B: Continue with rustpython-parser (ACCEPTABLE)**

- **Type stripping complexity**: Accept more complex manual implementation
- **Benefits**: Zero migration risk, proven stability
- **Trade-off**: More development effort for type stripping feature

**Option C: Full Migration (CONDITIONAL CONSIDERATION)**

- **Trigger**: Successful type stripping implementation with ruff AST
- **Timeline**: After validating hybrid approach
- **Benefits**: Full 2,500+ line reduction potential

### Strategic Timeline

**Immediate (Type Stripping Implementation)**: **Hybrid Approach** ✅ **NEW RECOMMENDATION**

- Use ruff AST specifically for type stripping implementation
- Validate ruff integration with limited scope
- Leverage Meta's production validation and built-in infrastructure

**Short-term (Post Type Stripping)**: **Evaluation Phase**

- Assess ruff AST integration experience from type stripping
- **IF** successful → consider broader migration
- **IF** challenging → maintain rustpython-parser for core bundling

**Long-term**: **Conditional Full Migration**

- Based on type stripping implementation experience
- Contingent on continued ruff API stability
- When development capacity allows comprehensive migration

### Bottom Line

The analysis reveals a **strategic shift** from the initial assessment:

**Original Conclusion**: Continue with rustpython-parser (maintenance concerns eliminated, API stability decisive)

**Updated Conclusion**: **Hybrid approach recommended** - Meta's production validation and type stripping advantages create compelling case for targeted ruff AST adoption.

The **hybrid approach balances** the substantial technical benefits of ruff AST for type stripping against the proven stability of rustpython-parser for core bundling logic, providing a low-risk path to evaluate ruff integration while delivering immediate value.

**Recommended Next Steps**:

1. Implement type stripping using ruff AST as proof of concept
2. Evaluate integration experience and stability
3. Make full migration decision based on practical results
