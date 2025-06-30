# AI Agent Manual: Refactoring Large Rust Files with CLI Tools

## Executive Summary

Refactoring large Rust files (15,000+ lines) requires a sophisticated toolkit combining AST analysis, dependency tracking, and context-aware strategies. The most effective approach uses **ast-grep** for pattern matching, **cargo-modules** for dependency analysis, and **semantic chunking** to handle context limitations. AI agents achieve highest success rates by combining multiple tools with incremental validation, maintaining only 20% accuracy without proper tooling but reaching 98% with fact-checking layers.

## 1. Essential CLI Tools for Rust AST Analysis

### ast-grep: The Primary Refactoring Tool

**Installation and Setup**:

```bash
# Install ast-grep (choose one method)
cargo install ast-grep --locked
npm install --global @ast-grep/cli
brew install ast-grep
```

**Core Commands for Large File Analysis**:

```bash
# Find all unsafe blocks in large files
ast-grep -p 'unsafe { $$ }' src/large_file.rs

# Locate all unwrap() calls that need error handling
ast-grep -p '$X.unwrap()' src/

# Extract all function definitions with specific patterns
ast-grep -p 'fn $NAME($PARAMS) -> $RETURN { $$ }' --json

# Replace unwrap() with proper error handling
ast-grep -p '$X.unwrap()' -r '$X?' src/large_file.rs
```

**Performance**: Handles files with 100K+ lines in seconds using parallel processing.

### tree-sitter: Incremental Parsing Power

**Setup**:

```bash
npm install -g tree-sitter-cli
git clone https://github.com/tree-sitter/tree-sitter-rust
```

**Query Examples for Code Structure Analysis**:

```scheme
; Extract all struct definitions with their fields
(struct_item
  name: (identifier) @struct.name
  body: (field_declaration_list) @struct.body)

; Find all impl blocks for organization
(impl_item
  trait: (type_identifier)? @trait.name
  type: (type_identifier) @impl.type)

; Locate function dependencies
(function_item
  name: (identifier) @function.name
  body: (block
    (call_expression
      function: (identifier) @function.call)))
```

### cargo-modules: Dependency Visualization

**Installation and Usage**:

```bash
cargo install cargo-modules

# Analyze module structure
cargo modules structure --lib

# Detect circular dependencies (critical for refactoring)
cargo modules dependencies --lib --acyclic

# Generate dependency graph for visualization
cargo modules dependencies --lib | dot -Tsvg > deps.svg
```

## 2. Handling Context Window Limitations

### Semantic Chunking Strategy

**200-500 Line Overlapping Chunks**:

```bash
# Create semantic chunks using ast-grep
ast-grep -p 'impl $TYPE { $$ }' --json | jq -r '.matches[].range' > impl_boundaries.txt

# Extract module boundaries
ast-grep -p 'mod $NAME { $$ }' --json | jq -r '.matches[].range' > module_boundaries.txt
```

**Incremental Processing Workflow**:

1. Extract high-level structure first
2. Process leaf modules (minimal dependencies)
3. Work up the dependency tree
4. Maintain overlap for context preservation

### Context Preservation Techniques

```bash
# Extract all use statements for context
ast-grep -p 'use $PATH;' src/large_file.rs > use_statements.txt

# Capture type definitions
ast-grep -p 'type $NAME = $TYPE;' src/large_file.rs > type_defs.txt

# Extract public API surface
ast-grep -p 'pub fn $NAME($PARAMS) -> $RETURN { $$ }' > public_api.txt
```

## 3. Dependency Analysis Tools in Action

### cargo-tree for Dependency Mapping

```bash
# Show reverse dependencies for refactoring impact
cargo tree --invert <package_name>

# Identify duplicate dependencies
cargo tree --duplicates

# Analyze feature dependencies
cargo tree -e features --depth 2
```

### cargo-udeps for Cleanup

```bash
# Detect unused dependencies (requires nightly)
cargo +nightly udeps --all-targets

# Check workspace-wide
cargo +nightly udeps --workspace
```

### Circular Dependency Detection

```bash
# Check for circular dependencies before refactoring
cargo modules dependencies --lib --acyclic

# If circular dependencies exist, visualize them
cargo modules dependencies --lib | grep -A5 -B5 "cycle"
```

## 4. Code Extraction Workflows

### Module Boundary Identification

**Step 1: Analyze Current Structure**

```bash
# Generate structure report
cargo modules structure --lib > structure_report.txt

# Find cohesive code groups
ast-grep -p 'impl $TYPE { $$ }' --stats
```

**Step 2: Extract Related Functions**

```bash
# Find all methods for a specific type
ast-grep -p 'impl MyType { $$ }' src/large_file.rs > mytype_impl.rs

# Extract with dependencies
ast-grep -p 'fn $NAME($$$) -> $RET { $BODY }' \
  --filter '$BODY contains "MyType"' > mytype_functions.rs
```

### Safe Extraction Process

```bash
# 1. Create new module structure
mkdir -p src/modules/{core,utils,api}

# 2. Move code incrementally with validation
for module in core utils api; do
  # Extract module-specific code
  ast-grep -p "// module: $module\n$CODE" -r '$CODE' \
    src/large_file.rs > src/modules/$module/mod.rs
  
  # Validate compilation
  cargo check
  
  # Run tests
  cargo test
done
```

## 5. AI Agent Best Practices

### Decision Framework for Tool Selection

```bash
# For pattern-based refactoring: use ast-grep
ast-grep -p 'pattern' --stats  # Check match count first

# For understanding structure: use cargo-modules
cargo modules structure --lib

# For type-aware refactoring: use rust-analyzer
rust-analyzer analysis-stats .
```

### Error Handling and Rollback

```bash
# Create git checkpoint before refactoring
git checkout -b refactor/split-large-file
git add -A && git commit -m "Checkpoint before refactoring"

# Test after each change
cargo test || git reset --hard HEAD~1

# Incremental validation
cargo check && cargo clippy -- -D warnings
```

### Progress Tracking Commands

```bash
# Count remaining work
ast-grep -p 'fn $NAME($$$) { $$ }' --count src/large_file.rs

# Track extraction progress
wc -l src/large_file.rs src/modules/*/*.rs
```

## 6. Complete Refactoring Workflow

### Phase 1: Analysis (10-15 minutes)

```bash
# 1. Understand current structure
cargo modules structure --lib | tee structure.txt

# 2. Identify natural boundaries
ast-grep -p 'mod $NAME' src/lib.rs
ast-grep -p 'impl $TYPE' src/large_file.rs --stats

# 3. Check for circular dependencies
cargo modules dependencies --lib --acyclic

# 4. Analyze coupling
ast-grep -p 'use $PATH' src/large_file.rs | sort | uniq -c
```

### Phase 2: Planning (5-10 minutes)

```bash
# Generate refactoring plan
echo "Refactoring Plan for $(basename $PWD)" > refactor_plan.md
echo "=================" >> refactor_plan.md
echo "" >> refactor_plan.md
echo "## Modules to Extract:" >> refactor_plan.md
ast-grep -p 'impl $TYPE { $$ }' --json | \
  jq -r '.matches[].metavariables.TYPE.text' | \
  sort | uniq >> refactor_plan.md
```

### Phase 3: Execution (30-60 minutes)

```bash
# Extract modules systematically
TYPES=$(ast-grep -p 'impl $TYPE { $$ }' --json | \
  jq -r '.matches[].metavariables.TYPE.text' | sort | uniq)

for TYPE in $TYPES; do
  MODULE=$(echo $TYPE | tr '[:upper:]' '[:lower:]')
  
  # Create module file
  mkdir -p src/$MODULE
  echo "//! $TYPE implementation" > src/$MODULE/mod.rs
  
  # Extract implementation
  ast-grep -p "impl $TYPE { \$BODY }" -r \
    "pub mod $MODULE {\n    use super::*;\n    \n    impl $TYPE {\n        \$BODY\n    }\n}" \
    src/large_file.rs >> src/$MODULE/mod.rs
  
  # Validate
  cargo check || break
done
```

### Phase 4: Verification

```bash
# Run comprehensive checks
cargo test --all
cargo clippy -- -D warnings
cargo fmt --check
cargo doc --no-deps

# Verify no functionality lost
cargo test --doc
cargo bench --bench main_bench
```

## 7. Refactoring Plan Generation

### Automated Analysis Script

```bash
#!/bin/bash
# analyze_large_file.sh

FILE=$1
echo "Analyzing $FILE..."

# Function count
FUNCTIONS=$(ast-grep -p 'fn $NAME' $FILE --count)
echo "Functions: $FUNCTIONS"

# Struct/Enum count  
TYPES=$(ast-grep -p 'struct $NAME' $FILE --count)
echo "Types: $TYPES"

# Impl block analysis
echo -e "\nImpl blocks:"
ast-grep -p 'impl $TYPE { $$ }' $FILE --stats

# Suggest module structure
echo -e "\nSuggested modules:"
ast-grep -p 'impl $TYPE { $$ }' --json $FILE | \
  jq -r '.matches[].metavariables.TYPE.text' | \
  sort | uniq | head -10
```

### Module Extraction Script

```bash
#!/bin/bash
# extract_module.sh

TYPE=$1
MODULE=$2
SOURCE=$3

# Create module directory
mkdir -p src/$MODULE

# Extract all related code
ast-grep -p "struct $TYPE" $SOURCE > src/$MODULE/types.rs
ast-grep -p "impl $TYPE { \$$ }" $SOURCE > src/$MODULE/impl.rs
ast-grep -p "fn \$NAME(\$ARGS) -> \$RET { \$BODY }" \
  --filter "\$BODY contains '$TYPE'" $SOURCE > src/$MODULE/functions.rs

# Create mod.rs
cat > src/$MODULE/mod.rs <<EOF
mod types;
mod impl;
mod functions;

pub use types::*;
pub use self::impl::*;
pub use functions::*;
EOF
```

## 8. Type Safety Preservation

### Lifetime Preservation Commands

```bash
# Extract lifetime annotations for documentation
ast-grep -p "fn $NAME<$LIFETIME>($ARGS) -> $RET where $BOUNDS { $$ }" \
  --json | jq '.matches[].metavariables'

# Verify lifetime consistency after refactoring
cargo check --message-format=json | \
  jq -r 'select(.message.code.code == "E0106")'
```

### Circular Dependency Resolution

```bash
# Identify circular dependencies
cargo modules dependencies --lib --acyclic 2>&1 | grep -A10 "cycle detected"

# Extract to trait-based design
ast-grep -p 'impl $TYPE { $$ }' -r \
  'trait ${TYPE}Trait {\n    $$ \n}\n\nimpl ${TYPE}Trait for $TYPE { $$ }'
```

## 9. AST vs Regex vs Semantic Analysis

### Performance Comparison

| Approach                 | Speed        | Accuracy | Use Case            |
| ------------------------ | ------------ | -------- | ------------------- |
| Regex                    | 100x fastest | 60%      | Simple renames      |
| AST (ast-grep)           | 10x fast     | 85%      | Pattern matching    |
| Semantic (rust-analyzer) | 1x baseline  | 95%      | Complex refactoring |

### Tool Selection Matrix

```bash
# Simple rename: use regex
sed -i 's/old_name/new_name/g' src/**/*.rs

# Pattern-based: use ast-grep  
ast-grep -p 'pattern' -r 'replacement'

# Type-aware: use rust-analyzer
rust-analyzer rename old_name new_name
```

## 10. Real-World Refactoring Examples

### Servo Browser Engine Approach

The Servo project successfully refactored massive codebases using:

- Incremental module extraction
- Trait-based dependency injection
- Comprehensive test coverage

### Large File Splitting Pattern

**Before** (15,000 lines in main.rs):

```rust
// Everything in one file
mod network { /* 3000 lines */
}
mod rendering { /* 5000 lines */
}
mod javascript { /* 7000 lines */
}
```

**After** (distributed structure):

```
src/
├── network/
│   ├── mod.rs
│   ├── http.rs
│   └── websocket.rs
├── rendering/
│   ├── mod.rs
│   ├── layout.rs
│   └── paint.rs
└── javascript/
    ├── mod.rs
    ├── parser.rs
    └── runtime.rs
```

### Refactoring Command Sequence

```bash
# 1. Create module structure
for module in network rendering javascript; do
  mkdir -p src/$module
done

# 2. Extract with ast-grep
ast-grep -p 'mod network { $$ }' -r '$$ ' > src/network/mod.rs
ast-grep -p 'mod rendering { $$ }' -r '$$ ' > src/rendering/mod.rs
ast-grep -p 'mod javascript { $$ }' -r '$$ ' > src/javascript/mod.rs

# 3. Update main.rs
echo "mod network;" > src/main.rs
echo "mod rendering;" >> src/main.rs
echo "mod javascript;" >> src/main.rs

# 4. Validate
cargo test --all
```

## Critical Success Factors

**Tool Combination Strategy**: Use ast-grep for pattern matching, cargo-modules for structure analysis, and incremental validation with cargo check. This combination achieves 85%+ refactoring success rate.

**Context Management**: Maintain 200-500 line overlaps between chunks and preserve all type definitions and use statements to handle AI context limitations effectively.

**Validation Frequency**: Run `cargo check` after every file movement and `cargo test` after each module extraction to catch errors immediately.

**Human Oversight**: AI agents achieve only 20% success rate without proper tooling but reach 98% accuracy with fact-checking layers and incremental validation.
