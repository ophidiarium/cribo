# Function-Scoped Import Rewriting: Implementation Documentation

## Executive Summary

This document describes the implemented enhancement to the cribo Python bundler that automatically resolves certain circular dependencies by rewriting module-level imports as function-scoped imports. This transformation breaks circular dependency cycles while preserving program semantics.

**Status**: ✅ Implemented and tested

## Problem Statement

### Initial Situation

The cribo bundler detected and classified circular dependencies but could not resolve them automatically. When circular dependencies were detected, the bundler failed with an error message suggesting manual intervention:

```
Error: Circular dependencies detected in the module graph:

Cycle 1: module_a → module_b → module_c → module_a
  Type: FunctionLevel
  Suggestion: Move imports inside functions to enable lazy loading
```

### The Challenge

Python's import system evaluates modules at import time, which means circular imports can cause `ImportError` when modules try to import each other during initialization:

```python
# module_a.py
from module_b import function_b

def function_a():
    return function_b() + 1

# module_b.py
from module_a import function_a  # ImportError: cannot import name 'function_a'

def function_b():
    return function_a() + 2
```

### The Solution

Many circular dependencies can be resolved by moving imports from module-level to function-level. This defers the import until the function is called, breaking the initialization-time cycle:

```python
# module_a.py - FIXED
def function_a():
    from module_b import function_b  # Import moved inside function
    return function_b() + 1

# module_b.py - FIXED
def function_b():
    from module_a import function_a  # Import moved inside function
    return function_a() + 2
```

## Implementation Details

### Circular Dependency Detection

The bundler successfully:

1. Detects circular dependencies using Tarjan's algorithm
2. Classifies them into types (FunctionLevel, ClassLevel, ImportTime, ModuleConstants)
3. Tracks import usage via `read_vars` (module-level) vs `eventual_read_vars` (function-level)

### Classification Logic

```rust
// In cribo_graph.rs
fn classify_cycle_type(
    &self,
    module_names: &[String],
    import_chain: &[ImportEdge],
) -> CircularDependencyType {
    // ...
    if analysis_result.imports_used_in_functions_only {
        CircularDependencyType::FunctionLevel // Can be resolved!
    }
    // ...
}
```

### Automatic Resolution

The bundler now automatically resolves function-level circular dependencies by rewriting imports.

## Implemented Solution

### High-Level Approach

1. **Identify Resolvable Imports**: During dependency analysis, track which imports can be safely moved to function scope
2. **AST Transformation**: Rewrite the AST to move qualifying imports inside the functions that use them
3. **Deduplication**: Ensure imports aren't duplicated if used in multiple functions
4. **Preserve Semantics**: Maintain program behavior and import side effects

### Implementation Architecture

#### Module Structure

The implementation consists of the following key components:

1. **`import_rewriter.rs`**: Core import rewriting logic
2. **`cribo_graph.rs`**: Enhanced circular dependency analysis
3. **`orchestrator.rs`**: Integration of rewriting into bundling workflow
4. **`code_generator.rs`**: Modified to handle modules with function-scoped imports

#### Import Tracking

The graph analysis tracks:

```rust
// In import_rewriter.rs
#[derive(Debug, Clone)]
pub struct MovableImport {
    /// The original import statement
    pub import_stmt: ImportStatement,
    /// Functions that use this import
    pub target_functions: Vec<String>,
    /// The source module containing this import
    pub source_module: String,
    /// Line number of the original import
    pub line_number: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImportStatement {
    /// Regular import: `import module` or `import module as alias`
    Import {
        module: String,
        alias: Option<String>,
    },
    /// From import: `from module import name` or `from module import name as alias`
    FromImport {
        module: Option<String>,
        names: Vec<(String, Option<String>)>,
        level: u32,
    },
}
```

#### Import Movement Eligibility

An import can be moved to function scope if:

1. It's only used within function bodies (not at module level)
2. It participates in a circular dependency
3. It doesn't have side effects
4. It's not used for class inheritance or decorators

#### AST Transformation

The implemented AST transformer:

```rust
pub struct ImportRewriter {
    /// Import deduplication strategy
    dedup_strategy: ImportDeduplicationStrategy,
}

impl ImportRewriter {
    /// Analyze a module graph to identify imports that can be moved to break cycles
    pub fn analyze_movable_imports(
        &mut self,
        graph: &CriboGraph,
        resolvable_cycles: &[CircularDependencyGroup],
    ) -> Vec<MovableImport>

    /// Rewrite a module's AST to move imports into function scope
    pub fn rewrite_module(
        &mut self,
        module_ast: &mut ModModule,
        movable_imports: &[MovableImport],
        module_name: &str,
    ) -> Result<()>
}
```

#### Import Placement Strategy

The current implementation uses the **function start** strategy:

```python
# Imports are placed at the beginning of function bodies
def function_a():
    from module_b import function_b  # Import at function start
    return function_b() + 1
```

This strategy is simple and reliable. Future enhancements could implement more sophisticated placement strategies.

### Integration with Bundling Workflow

#### Orchestrator Integration

The import rewriting is integrated into the bundling workflow in `orchestrator.rs`:

```rust
// In emit_static_bundle
if let Some(analysis) = params.circular_dep_analysis {
    if !analysis.resolvable_cycles.is_empty() {
        info!("Applying function-scoped import rewriting to resolve circular dependencies");
        let mut import_rewriter = ImportRewriter::new(ImportDeduplicationStrategy::FunctionStart);
        let movable_imports = import_rewriter.analyze_movable_imports(
            params.graph, 
            &analysis.resolvable_cycles
        );
        
        for (module_name, ast, _, _) in &mut module_asts {
            import_rewriter.rewrite_module(ast, &movable_imports, module_name)?;
        }
    }
}
```

#### Code Generator Adaptation

The code generator was modified to handle modules with function-scoped imports:

```rust
// In code_generator.rs
fn find_modules_with_function_imports(
    &self,
    modules: &[(String, ModModule, PathBuf, String)],
) -> FxIndexSet<String> {
    let mut modules_with_function_imports = FxIndexSet::default();
    for (module_name, ast, _, _) in modules {
        if self.module_has_function_scoped_imports(ast) {
            log::info!("Module '{}' has function-scoped imports", module_name);
            modules_with_function_imports.insert(module_name.clone());
        }
    }
    modules_with_function_imports
}
```

Modules with function-scoped imports are forced to use the wrapper module approach instead of being inlined, ensuring that the module objects exist when the function-scoped imports execute.

### Edge Cases and Considerations

#### 1. Import Side Effects

Some imports have side effects that must execute at module load time:

```python
# config.py
import os
os.environ['CONFIG_LOADED'] = 'true'  # Side effect!

# This cannot be moved to function scope
```

#### 2. Performance Implications

Function-scoped imports have a small performance cost on repeated calls:

```python
def frequently_called():
    from heavy_module import func  # Re-evaluated each call
    return func()
```

Solution: Add a configuration option to control optimization vs compatibility.

#### 3. Type Annotations

Imports used in type annotations need special handling:

```python
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from module_b import ClassB  # Only for type checking

def function_a(param: 'ClassB'):  # String annotation
    from module_b import ClassB  # Runtime import
    return isinstance(param, ClassB)
```

#### 4. Import Variations

Handle all import forms:

```python
# Simple import
import module_b
# From import
from module_b import func
# Aliased import
from module_b import func as b_func
# Star import (generally avoid moving these)
from module_b import *
```

### Testing Strategy

#### Test Framework Enhancement

A new test prefix `pyfail_` was introduced to properly test circular dependency resolution:

- **`pyfail_` fixtures**: Must fail direct Python execution but succeed after bundling
- **`xfail_` fixtures**: Must pass direct execution but fail bundling or produce different output
- **Normal fixtures**: Must pass both original and bundled execution with matching output

#### Implemented Tests

The following circular dependency tests now pass with import rewriting:

1. **`pyfail_three_module_cycle`**: Three modules with circular imports
   ```python
   # Module A → Module B → Module C → Module A
   ```

2. **`pyfail_four_module_cycle`**: Four modules with circular imports
   ```python
   # Module A → Module B → Module C → Module D → Module A
   ```

3. **`pyfail_package_level_cycles`**: Package-level circular dependencies
   ```python
   # pkg1.main → pkg2.helper → pkg1.utility → pkg1.main
   ```

4. **`pyfail_relative_import_cycles`**: Relative imports with circular dependencies
   ```python
   # services.auth → services.database → services.auth
   ```

All tests demonstrate successful resolution of circular dependencies through automatic import rewriting.

### Configuration Options

The feature is automatically enabled when circular dependencies are detected. No configuration is required for basic usage.

Future configuration options could include:

```toml
[bundler]
# Strategy for placing imports in functions
import_placement_strategy = "function_start" # or "before_use"

# List of modules where imports should not be moved (side effects)
preserve_module_imports = ["config", "settings"]
```

### Achieved Results

1. **Functionality**: Successfully resolves FunctionLevel circular dependencies automatically
2. **Compatibility**: Bundled code executes correctly with imports moved to function scope
3. **Performance**: Minimal impact - imports are cached by Python after first execution
4. **Usability**: Zero configuration required - feature activates automatically when needed

## Implementation Summary

The function-scoped import rewriting feature has been successfully implemented and tested. Key achievements:

- **Automatic Detection**: Leverages existing circular dependency analysis to identify resolvable cycles
- **Smart Rewriting**: Moves only the imports that are safe to move and necessary to break cycles
- **Module Handling**: Ensures modules with function-scoped imports use wrapper approach for proper execution
- **Comprehensive Testing**: Test framework enhanced with `pyfail_` prefix for circular dependency tests

## Future Enhancements

1. **Import Placement Strategies**: Support for placing imports just before first use
2. **Performance Optimization**: Cache frequently used imports at module level when safe
3. **Configuration Options**: Allow users to exclude specific modules from rewriting
4. **Side Effect Detection**: Enhanced analysis to detect imports with side effects

## Discovered Limitation: Function-Level Import Discovery

### The Problem

During testing with the `mixed_import_patterns` fixture, a critical limitation was discovered: **the bundler only discovers module-level imports and completely misses function-scoped imports during the dependency discovery phase**.

This means that if a module is **only** imported at function level (not at module level anywhere in the codebase), it will not be included in the bundle at all.

#### Example Demonstrating the Issue

```python
# main.py
def main():
    # This import is NOT discovered by the bundler
    from config import Config  
    config = Config()
    
# config.py
class Config:
    pass
    
# Result: config.py is not bundled, causing ImportError at runtime
```

### Root Cause Analysis

The limitation stems from the `extract_imports` method in `orchestrator.rs`:

```rust
// Current implementation only examines top-level statements
pub fn extract_imports(
    &self,
    file_path: &Path,
    resolver: Option<&mut ModuleResolver>,
) -> Result<Vec<String>> {
    // ... parse file ...

    // PROBLEM: Only iterates module-level statements
    for stmt in parsed.syntax().body.iter() {
        self.extract_imports_from_statement(stmt, &mut context);
    }

    Ok(imports)
}
```

The method only examines `parsed.syntax().body`, which contains module-level statements. It never traverses into:

- Function bodies (`Stmt::FunctionDef`)
- Class bodies (`Stmt::ClassDef`)
- Conditional blocks (`Stmt::If`, `Stmt::While`)
- Any nested scopes where imports might exist

### Impact

This limitation affects:

1. **Modern Python patterns**: Many codebases use function-scoped imports to avoid circular dependencies
2. **Lazy loading**: Modules imported only when needed are missed
3. **Conditional imports**: Platform-specific imports inside functions are not discovered
4. **Import rewriting effectiveness**: The feature can move imports to functions, but if those imports aren't discovered elsewhere, the modules won't be bundled

## Proposed Architectural Fix

### High-Level Solution

Implement a comprehensive AST visitor that recursively traverses all scopes to discover imports at any nesting level.

### Detailed Design

#### 1. Enhanced Import Discovery with Scope Tracking

```rust
#[derive(Debug, Clone)]
pub struct ScopedImport {
    pub module_name: String,
    pub scope: ImportScope,
    pub line_number: usize,
}

#[derive(Debug, Clone)]
pub enum ImportScope {
    Module,
    Function(String),       // Function name
    Class(String),          // Class name
    Method(String, String), // (Class name, method name)
    Nested(Vec<String>),    // Arbitrary nesting
}
```

#### 2. AST Visitor Implementation

```rust
struct ImportCollector {
    imports: Vec<ScopedImport>,
    current_scope: Vec<ScopeElement>,
}

#[derive(Clone)]
enum ScopeElement {
    Function(String),
    Class(String),
    If,
    While,
    With,
    Try,
}

impl ImportCollector {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Import(import_stmt) => {
                self.collect_import(import_stmt);
            }
            Stmt::ImportFrom(import_from) => {
                self.collect_import_from(import_from);
            }
            Stmt::FunctionDef(func) => {
                self.current_scope
                    .push(ScopeElement::Function(func.name.to_string()));
                for stmt in &func.body {
                    self.visit_stmt(stmt);
                }
                self.current_scope.pop();
            }
            Stmt::ClassDef(class) => {
                self.current_scope
                    .push(ScopeElement::Class(class.name.to_string()));
                for stmt in &class.body {
                    self.visit_stmt(stmt);
                }
                self.current_scope.pop();
            }
            Stmt::If(if_stmt) => {
                self.current_scope.push(ScopeElement::If);
                for stmt in &if_stmt.body {
                    self.visit_stmt(stmt);
                }
                for elif in &if_stmt.elif_else_clauses {
                    for stmt in &elif.body {
                        self.visit_stmt(stmt);
                    }
                }
                self.current_scope.pop();
            }
            Stmt::While(while_stmt) => {
                self.current_scope.push(ScopeElement::While);
                for stmt in &while_stmt.body {
                    self.visit_stmt(stmt);
                }
                self.current_scope.pop();
            }
            Stmt::With(with_stmt) => {
                self.current_scope.push(ScopeElement::With);
                for stmt in &with_stmt.body {
                    self.visit_stmt(stmt);
                }
                self.current_scope.pop();
            }
            Stmt::Try(try_stmt) => {
                self.visit_try_stmt(try_stmt);
            }
            Stmt::For(for_stmt) => {
                for stmt in &for_stmt.body {
                    self.visit_stmt(stmt);
                }
            }
            // ... handle other statement types
            _ => {}
        }
    }

    fn current_scope_type(&self) -> ImportScope {
        // Convert current_scope stack to ImportScope enum
        if self.current_scope.is_empty() {
            ImportScope::Module
        } else {
            // Build appropriate ImportScope based on stack
            match &self.current_scope[..] {
                [ScopeElement::Function(name)] => ImportScope::Function(name.clone()),
                [ScopeElement::Class(cls), ScopeElement::Function(method)] => {
                    ImportScope::Method(cls.clone(), method.clone())
                }
                // ... other patterns
                _ => ImportScope::Nested(self.scope_path()),
            }
        }
    }
}
```

#### 3. Integration Points

Replace the current `extract_imports` with the enhanced version:

```rust
impl BundleOrchestrator {
    /// Extract ALL imports from a Python file, including function-scoped
    pub fn extract_all_imports(
        &self,
        file_path: &Path,
        resolver: Option<&mut ModuleResolver>,
    ) -> Result<Vec<String>> {
        let source = fs::read_to_string(file_path)?;
        let parsed = ruff_python_parser::parse_module(&source)?;

        let mut collector = ImportCollector::new();
        collector.visit_module(&parsed.syntax());

        // Extract unique module names
        let mut imports: IndexSet<String> = IndexSet::new();
        for scoped_import in collector.imports {
            imports.insert(scoped_import.module_name);
        }

        Ok(imports.into_iter().collect())
    }
}
```

#### 4. Modify Discovery Phase

Update `build_dependency_graph` to use the new import discovery:

```rust
// In build_dependency_graph, line ~526
let imports = self.extract_all_imports(&module_path, Some(params.resolver))?;
```

### Benefits of This Approach

1. **Complete Discovery**: All imports are found regardless of scope
2. **Maintains Compatibility**: Existing module-level import handling unchanged
3. **Future Enhancements**: Scope information enables smarter optimizations
4. **Better Analysis**: Can generate reports on import patterns and usage

### Implementation Considerations

1. **Performance**: Recursive AST traversal has minimal overhead compared to parsing
2. **Memory**: Scope tracking adds negligible memory usage
3. **Correctness**: Must handle all Python AST node types that can contain statements
4. **Testing**: Existing fixtures like `mixed_import_patterns` will automatically validate the fix

### Estimated Implementation Effort

- **AST Visitor Framework**: 0.5-1 day
- **Import Collection Logic**: 0.5 day
- **Integration & Testing**: 0.5-1 day
- **Edge Cases & Polish**: 0.5 day

**Total**: 2-3 days of focused development

### Future Enhancements Enabled

With complete import discovery, additional features become possible:

1. **Import Usage Analysis**: Track which functions use which imports
2. **Dead Import Detection**: Find imports that are never used
3. **Import Optimization**: Move imports to narrowest necessary scope
4. **Conditional Import Handling**: Smart bundling based on import conditions

## Conclusion

Function-scoped import rewriting is now a core feature of cribo, automatically resolving circular dependencies without user intervention. However, the discovered limitation in import discovery prevents the bundler from handling modules that are only imported at function level.

The proposed architectural fix would make cribo's import discovery truly comprehensive, enabling it to handle all modern Python import patterns while maintaining backward compatibility. This enhancement would complete the circular dependency resolution feature and make cribo suitable for a wider range of Python codebases.
