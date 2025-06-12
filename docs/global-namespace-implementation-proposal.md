# Technical Implementation Proposal: Global Namespace Support in Cribo

## Problem Statement

The current module bundling approach wraps modules in init functions, which transforms module-level variables into function-local variables. This breaks Python's `global` statement semantics, causing `NameError` when functions try to access or modify module-level variables using `global`.

## Proposed Solution: Module Globals Dictionary Pattern

### Overview

Transform global variable access to use a module-specific globals dictionary that preserves Python's global semantics within the bundled context.

### Detailed Implementation Plan

#### 1. Semantic Analysis Enhancement

```rust
// In semantic_bundler.rs
#[derive(Debug, Clone)]
pub struct ModuleGlobalInfo {
    /// Variables that exist at module level
    pub module_level_vars: FxIndexSet<String>,

    /// Variables declared with 'global' keyword
    pub global_declarations: FxIndexMap<String, Vec<TextRange>>,

    /// Locations where globals are read
    pub global_reads: FxIndexMap<String, Vec<TextRange>>,

    /// Locations where globals are written
    pub global_writes: FxIndexMap<String, Vec<TextRange>>,

    /// Functions that use global statements
    pub functions_using_globals: FxIndexSet<String>,
}

impl SemanticBundler {
    pub fn analyze_module_globals(
        &mut self,
        module_id: ModuleId,
        ast: &ModModule,
    ) -> ModuleGlobalInfo {
        let mut info = ModuleGlobalInfo::default();

        // First pass: collect module-level variables
        for stmt in &ast.body {
            match stmt {
                Stmt::Assign(assign) => {
                    for target in &assign.targets {
                        if let Expr::Name(name) = target {
                            info.module_level_vars.insert(name.id.to_string());
                        }
                    }
                }
                Stmt::AnnAssign(ann_assign) => {
                    if let Expr::Name(name) = &ann_assign.target {
                        info.module_level_vars.insert(name.id.to_string());
                    }
                }
                _ => {}
            }
        }

        // Second pass: analyze global usage in functions
        GlobalUsageVisitor::new(&mut info).visit_module(ast);

        info
    }
}
```

#### 2. AST Transformation

```rust
// In code_generator.rs
impl CodeGenerator {
    fn transform_module_with_globals(
        &mut self,
        module: &mut ModModule,
        global_info: &ModuleGlobalInfo,
    ) -> Result<()> {
        // Step 1: Create __module_globals__ initialization
        let globals_init = self.create_globals_dict_init(&global_info.module_level_vars);

        // Step 2: Transform global statements to nonlocal
        GlobalStatementTransformer::new(&global_info).transform_module(module)?;

        // Step 3: Rewrite variable access to use dictionary
        GlobalAccessRewriter::new(&global_info).rewrite_module(module)?;

        Ok(())
    }

    fn create_globals_dict_init(&self, vars: &FxIndexSet<String>) -> Stmt {
        // Generate: __module_globals__ = {'var1': value1, 'var2': value2, ...}
        let mut items = vec![];

        for var in vars {
            items.push((
                Some(Box::new(Expr::StringLiteral(ExprStringLiteral {
                    value: StringLiteralValue::single(StringLiteral {
                        value: var.clone().into(),
                        flags: StringLiteralFlags::empty(),
                        range: TextRange::default(),
                    }),
                    range: TextRange::default(),
                }))),
                Box::new(Expr::Name(ExprName {
                    id: var.clone(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
            ));
        }

        Stmt::Assign(StmtAssign {
            targets: vec![Expr::Name(ExprName {
                id: "__module_globals__".into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Dict(ExprDict {
                items,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }
}

struct GlobalAccessRewriter<'a> {
    global_info: &'a ModuleGlobalInfo,
    in_function: bool,
    current_function_globals: FxIndexSet<String>,
}

impl<'a> GlobalAccessRewriter<'a> {
    fn rewrite_name_access(&mut self, expr: &mut Expr) {
        if let Expr::Name(name_expr) = expr {
            if self.in_function && self.current_function_globals.contains(&name_expr.id) {
                // Transform: x → __module_globals__['x']
                *expr = Expr::Subscript(ExprSubscript {
                    value: Box::new(Expr::Name(ExprName {
                        id: "__module_globals__".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    slice: Box::new(Expr::StringLiteral(ExprStringLiteral {
                        value: StringLiteralValue::single(StringLiteral {
                            value: name_expr.id.clone().into(),
                            flags: StringLiteralFlags::empty(),
                            range: TextRange::default(),
                        }),
                        range: TextRange::default(),
                    })),
                    ctx: name_expr.ctx.clone(),
                    range: TextRange::default(),
                });
            }
        }
    }
}
```

#### 3. Module Wrapper Generation

```python
# Generated wrapper structure
def __cribo_init_module():
    # Original module-level code
    result = "base_result"
    process = "base_process_string"
    
    # Create module globals dictionary
    __module_globals__ = {
        'result': result,
        'process': process,
    }
    
    # Transform functions that use globals
    def initialize():
        # Original: global result
        # Transformed: use __module_globals__
        __module_globals__['result'] = f"initialized_{__module_globals__['result']}"
        return __module_globals__['result']
    
    # Update module object with current values
    module.result = __module_globals__['result']
    module.process = __module_globals__['process']
    module.__module_globals__ = __module_globals__  # For debugging
```

### Implementation Phases

#### Phase 1: Basic Global Support (1-2 days)

1. Implement `ModuleGlobalInfo` structure
2. Add basic global usage analysis
3. Create simple dictionary-based transformation
4. Fix the comprehensive_ast_rewrite test

#### Phase 2: Complete Transformation (3-5 days)

1. Handle all global access patterns
2. Support nested function scopes
3. Handle edge cases (comprehensions, lambdas)
4. Comprehensive test suite

#### Phase 3: Optimization (2-3 days)

1. Eliminate unnecessary transformations
2. Optimize dictionary access patterns
3. Dead global elimination
4. Performance benchmarking

### Alternative Approaches Considered

1. **Exec-based approach**: Use `exec()` with custom globals
   - Pros: Simpler implementation
   - Cons: Performance overhead, security concerns

2. **Class-based encapsulation**: Wrap module in a class
   - Pros: Clean namespace isolation
   - Cons: Changes module interface significantly

3. **AST manipulation to eliminate globals**: Convert to explicit parameters
   - Pros: No runtime overhead
   - Cons: Complex implementation, may break dynamic code

### Test Strategy

1. **Unit Tests**
   ```python
   # Test basic global modification
   def test_simple_global():
       global x
       x = 42

   # Test cross-function globals
   def test_cross_function_global():
       global shared
       shared = "initial"
       
       def modifier():
           global shared
           shared = "modified"
       
       modifier()
       assert shared == "modified"
   ```

2. **Integration Tests**
   - Comprehensive_ast_rewrite fixture
   - Real-world module examples
   - Performance benchmarks

### Success Criteria

1. All global statement patterns work correctly
2. No performance regression > 5%
3. Maintains deterministic output
4. Passes all existing tests
5. comprehensive_ast_rewrite test passes

### Risks and Mitigations

1. **Risk**: Complex edge cases with exec/eval
   - **Mitigation**: Document limitations, provide escape hatch

2. **Risk**: Performance overhead of dictionary access
   - **Mitigation**: Optimize hot paths, consider caching

3. **Risk**: Breaking changes to module interface
   - **Mitigation**: Maintain backward compatibility layer

## Recommendation

Proceed with the Module Globals Dictionary Pattern as it provides the best balance of correctness, maintainability, and performance. The implementation can be done incrementally, starting with basic support to fix the immediate test failure, then expanding to handle all edge cases.
