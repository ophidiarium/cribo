# Semantic Import Analysis for Cribo

Based on analysis of Ruff's AST semantic analysis implementation, here's a design for implementing Option 5 to fix the mixed import patterns with circular dependencies issue.

## Key Insights from Ruff

Ruff uses a sophisticated semantic analysis approach that tracks:

1. **Execution Context**: Each binding and reference has an associated context (Runtime vs Typing)
2. **Semantic Flags**: During AST traversal, flags track the current context (e.g., TYPING_CONTEXT, TYPE_CHECKING_BLOCK)
3. **Deferred Analysis**: Function bodies and type annotations are deferred and visited later with proper context
4. **Scope Tracking**: Each binding knows its scope, and references are resolved to specific bindings

## Proposed Implementation for Cribo

### 1. Add Execution Context Tracking

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionContext {
    /// Code that executes at module import time
    ModuleLevel,
    /// Code inside function/method bodies (deferred execution)
    FunctionBody,
    /// Code inside class bodies (executes during class definition)
    ClassBody,
    /// Code inside type annotations (may not execute at runtime)
    TypeAnnotation,
}
```

### 2. Enhanced Import Tracking

```rust
#[derive(Debug, Clone)]
pub struct ImportUsage {
    /// The name being imported
    pub name: String,
    /// Where this import is used
    pub usage_context: ExecutionContext,
    /// The location of the usage
    pub location: TextRange,
    /// Whether this usage requires runtime availability
    pub requires_runtime: bool,
}

#[derive(Debug, Clone)]
pub struct EnhancedImportInfo {
    /// Original import information
    pub base: ImportInfo,
    /// All usages of this import
    pub usages: Vec<ImportUsage>,
    /// Whether any usage requires runtime availability
    pub has_runtime_usage: bool,
    /// Whether all usages are in deferred contexts
    pub is_deferred_only: bool,
}
```

### 3. AST Visitor with Context Tracking

```rust
struct SemanticImportVisitor {
    /// Current execution context
    current_context: ExecutionContext,
    /// Stack of contexts for nested scopes
    context_stack: Vec<ExecutionContext>,
    /// Import name to usage tracking
    import_usages: HashMap<String, Vec<ImportUsage>>,
}

impl SemanticImportVisitor {
    fn visit_function_def(&mut self, name: &str, body: &[Stmt]) {
        // Function bodies are deferred
        self.push_context(ExecutionContext::FunctionBody);
        for stmt in body {
            self.visit_stmt(stmt);
        }
        self.pop_context();
    }

    fn visit_class_def(&mut self, name: &str, body: &[Stmt]) {
        // Class bodies execute during class definition
        self.push_context(ExecutionContext::ClassBody);
        for stmt in body {
            self.visit_stmt(stmt);
        }
        self.pop_context();
    }

    fn visit_name(&mut self, name: &str, location: TextRange) {
        // Track usage of potentially imported names
        if let Some(import_name) = self.resolve_import(name) {
            self.import_usages
                .entry(import_name)
                .or_default()
                .push(ImportUsage {
                    name: name.to_string(),
                    usage_context: self.current_context,
                    location,
                    requires_runtime: self.requires_runtime_availability(),
                });
        }
    }

    fn requires_runtime_availability(&self) -> bool {
        matches!(
            self.current_context,
            ExecutionContext::ModuleLevel | ExecutionContext::ClassBody
        )
    }
}
```

### 4. Integration with Circular Dependency Detection

```rust
impl DependencyGraph {
    pub fn add_edge_with_context(
        &mut self,
        from: &str,
        to: &str,
        import_info: EnhancedImportInfo,
    ) -> Result<(), DependencyGraphError> {
        // Only add edge if import has runtime usage
        if import_info.has_runtime_usage {
            self.add_edge(from, to, import_info.base)?;
        }
        Ok(())
    }

    pub fn find_circular_dependencies_semantic(&self) -> Vec<Vec<String>> {
        // Only consider edges with runtime dependencies
        let runtime_edges: Vec<_> = self
            .edges
            .iter()
            .filter(|e| {
                self.nodes[e.from]
                    .imports
                    .iter()
                    .any(|i| i.has_runtime_usage)
            })
            .collect();

        // Run SCC algorithm on runtime-only edges
        self.find_sccs_from_edges(&runtime_edges)
    }
}
```

### 5. Updated Bundling Logic

```rust
pub fn extract_imports_semantic(source: &str, filename: &str) -> Result<ExtractedModule> {
    let parsed = parse_python_source(source, filename)?;

    // First pass: collect all imports
    let mut import_visitor = ImportVisitor::new();
    import_visitor.visit_body(&parsed.body);

    // Second pass: track usage contexts
    let mut semantic_visitor = SemanticImportVisitor::new();
    semantic_visitor.visit_body(&parsed.body);

    // Combine import info with usage info
    let enhanced_imports = import_visitor
        .imports
        .into_iter()
        .map(|import| {
            let usages = semantic_visitor.get_usages(&import.name);
            let has_runtime_usage = usages.iter().any(|u| u.requires_runtime);
            let is_deferred_only = usages.iter().all(|u| !u.requires_runtime);

            EnhancedImportInfo {
                base: import,
                usages,
                has_runtime_usage,
                is_deferred_only,
            }
        })
        .collect();

    Ok(ExtractedModule {
        imports: enhanced_imports,
        // ... rest of the module data
    })
}
```

## Benefits

1. **Accurate Circular Dependency Detection**: Only runtime dependencies participate in cycle detection
2. **Better Error Messages**: Can explain why an import is considered a runtime dependency
3. **Optimization Opportunities**: Can defer or inline modules that are only used in function bodies
4. **Type Annotation Support**: Properly handles imports used only in type annotations

## Implementation Steps

1. Add `ExecutionContext` enum and `ImportUsage` struct
2. Implement `SemanticImportVisitor` that tracks context during AST traversal
3. Update `extract_imports` to use semantic analysis
4. Modify `DependencyGraph` to consider execution context
5. Update circular dependency detection to only consider runtime edges
6. Add tests for mixed import patterns with proper semantic analysis

## Example Resolution

For the problematic case:

```python
# module_a.py
from module_b import helper  # Used only in function body

def process():
    return helper()  # Deferred execution

# module_b.py
from module_a import process  # Used at module level

result = process()  # Runtime execution
```

The semantic analysis would determine:

- `module_a` imports `helper` with `is_deferred_only = true`
- `module_b` imports `process` with `has_runtime_usage = true`
- No runtime circular dependency exists
- Bundle order: `module_a`, then `module_b`
