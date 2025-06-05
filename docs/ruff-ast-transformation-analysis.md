# Ruff AST Transformation Patterns: Analysis for Serpen

## Executive Summary

After analyzing ruff's AST transformation patterns, there are significant architectural differences between ruff's approach and Serpen's current implementation. Ruff uses a more robust, type-safe visitor pattern with separate traits for read-only traversal and mutation, while Serpen uses a more manual approach.

## Key Architectural Differences

### 1. Visitor Pattern Architecture

#### Ruff's Approach

- **Two distinct traits**: `Visitor<'a>` for read-only traversal and `Transformer` for mutations
- **Comprehensive walk functions**: Every AST node type has a corresponding `walk_*` function
- **Type-safe traversal**: Each visitor method receives strongly-typed AST nodes
- **Evaluation order**: Visits nodes in evaluation order by default
- **Alternative visitors**: Provides `SourceOrderVisitor` for source-order traversal

#### Serpen's Current Approach

- Manual AST traversal with match statements
- No formal visitor pattern implementation
- Mixed concerns between traversal and transformation logic
- Less systematic approach to node visitation

### 2. AST Transformation Patterns

#### Ruff's Transformer Trait

```rust
pub trait Transformer {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        walk_stmt(self, stmt);
    }
    // ... methods for each node type
}
```

- Takes mutable references to AST nodes
- Default implementations call walk functions
- Allows selective overriding of specific node types
- Maintains traversal logic separate from transformation logic

#### Serpen's Current Pattern

- Direct mutation of AST nodes within match statements
- No separation between traversal and transformation
- Manual recursion for nested structures

### 3. Import Manipulation

#### Ruff's Approach

- Uses `libcst_native` for CST-based transformations
- Preserves formatting and comments
- `Importer` struct manages import modifications:
  - Tracks runtime vs type-checking imports
  - Handles import insertion with proper positioning
  - Manages symbol resolution and name conflicts
- Codemods for import manipulation:
  - `remove_imports`: Removes specific imports while preserving structure
  - `retain_imports`: Keeps only specified imports
  - Handles edge cases like trailing commas and comments

#### Serpen's Approach

- Direct AST manipulation without CST
- Manual tracking of import aliases and conflicts
- Less sophisticated handling of formatting preservation

### 4. Source Location Management

#### Ruff's Approach

- `relocate_expr` function for updating expression locations
- Uses a `Relocator` transformer that systematically updates all ranges
- Preserves source mapping through transformations
- `SourceMap` tracking for fix applications

#### Serpen's Approach

- No systematic approach to location management
- Manual range updates where needed

### 5. Performance Characteristics

#### Ruff's Approach

- Efficient visitor pattern with minimal allocations
- Walk functions are inlined for performance
- Transformations are applied in a single pass where possible
- CST parsing only when needed for precise edits

#### Serpen's Approach

- Multiple passes over AST for different operations
- Potential for redundant traversals

## Import-Specific Transformation Examples

### Ruff's Import Removal Pattern

```rust
pub(crate) fn remove_imports<'a>(
    member_names: impl Iterator<Item = &'a str>,
    stmt: &Stmt,
    locator: &Locator,
    stylist: &Stylist,
) -> Result<Option<String>> {
    // 1. Parse to CST for precise manipulation
    let module_text = locator.slice(stmt);
    let mut tree = match_statement(module_text)?;
    
    // 2. Extract import aliases
    let aliases = /* extract from CST */;
    
    // 3. Preserve formatting (trailing commas, comments)
    let trailing_comma = aliases.last().and_then(|alias| alias.comma.clone());
    
    // 4. Remove specified imports
    aliases.retain(|alias| /* keep if not in removal list */);
    
    // 5. Restore formatting
    if let Some(alias) = aliases.last_mut() {
        alias.comma = trailing_comma;
    }
    
    // 6. Generate code with proper styling
    Ok(Some(tree.codegen_stylist(stylist)))
}
```

### Ruff's Import Insertion Pattern

```rust
impl Importer {
    pub(crate) fn add_import(&self, import: &NameImport, at: TextSize) -> Edit {
        if let Some(stmt) = self.preceding_import(at) {
            // Insert after the last import
            Insertion::end_of_statement(stmt, self.locator, self.stylist)
                .into_edit(&required_import)
        } else {
            // Insert at start of file
            Insertion::start_of_file(self.python_ast, self.locator, self.stylist)
                .into_edit(&required_import)
        }
    }
}
```

## Chain of Transformations

Ruff chains transformations through:

1. **Diagnostic collection** → Identifies issues requiring fixes
2. **Fix generation** → Creates edits with proper applicability levels
3. **Fix sorting** → Orders fixes by position and rule priority
4. **Fix application** → Applies non-overlapping fixes in order
5. **Source mapping** → Maintains mapping between original and fixed code

## Recommendations for Serpen

### 1. Adopt Visitor Pattern

Implement ruff's dual visitor/transformer pattern:

- Create `Visitor` trait for analysis passes
- Create `Transformer` trait for modification passes
- Generate walk functions for all AST node types

### 2. Improve Import Handling

- Consider using CST for import modifications to preserve formatting
- Implement proper import insertion logic with position detection
- Add support for TYPE_CHECKING blocks

### 3. Systematic Location Management

- Implement a `Relocator` transformer for systematic range updates
- Maintain source mapping through transformations

### 4. Performance Optimization

- Reduce number of AST passes by combining operations
- Use visitor pattern to avoid redundant traversals
- Consider caching symbol tables across passes

### 5. Error Handling

- Adopt ruff's approach of graceful degradation
- Implement conflict detection for overlapping edits
- Add validation for transformed AST

## Migration Strategy

1. **Phase 1**: Implement basic visitor/transformer traits
2. **Phase 2**: Migrate existing traversal logic to use visitors
3. **Phase 3**: Adopt CST-based import manipulation
4. **Phase 4**: Implement comprehensive location management
5. **Phase 5**: Optimize performance with single-pass transformations

## Conclusion

Ruff's AST transformation patterns provide a more robust, maintainable, and performant approach compared to Serpen's current implementation. The key advantages are:

- Separation of concerns between traversal and transformation
- Type-safe visitor pattern
- Sophisticated import manipulation with formatting preservation
- Systematic source location management
- Better performance characteristics

Adopting these patterns would significantly improve Serpen's code quality and maintainability.
