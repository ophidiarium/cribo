# ExecutionStep Architecture Design

## Overview

This document describes the implemented architecture for the bundling execution pipeline in Cribo. The design follows a clean "Compiler → Bytecode → VM" pattern with strict separation of concerns between semantic analysis, compilation, and mechanical execution.

## Core Principles

1. **Single Responsibility**: Each component has exactly one job
2. **Compiler/VM Separation**: Intelligence lives in the compiler, execution is purely mechanical
3. **Clean Interfaces**: BundleProgram is the only data structure passed between compiler and VM
4. **Deterministic Output**: All operations produce reproducible results
5. **No State Leakage**: Compilation state never reaches the execution phase

## Architecture Flow

```
AnalysisResults → BundleCompiler → BundleProgram → Bundle VM → ModModule
   (semantic)       (compiler)       (bytecode)      (executor)   (output)
```

## Core Data Structures

### BundleCompiler (Stateful Compilation)

```rust
pub struct BundleCompiler<'a> {
    // Input context
    analysis_results: &'a AnalysisResults,
    graph: &'a CriboGraph,
    registry: &'a ModuleRegistry,
    entry_module_id: ModuleId,

    // Compilation state (never exposed)
    symbol_renames: IndexMap<GlobalBindingId, String>,
    live_items: FxHashMap<ModuleId, Vec<ItemId>>,
    classified_imports: FxHashMap<(ModuleId, ItemId), ImportClassification>,
    module_metadata: FxHashMap<ModuleId, ModuleMetadata>,
    module_aliases: FxHashMap<(ModuleId, String), ModuleId>,
    semantic_provider: Option<&'a SemanticModelProvider<'a>>,
}
```

### BundleProgram (Clean Output)

```rust
pub struct BundleProgram {
    /// Linear sequence of instructions
    pub steps: Vec<ExecutionStep>,

    /// AST node renames for CopyStatement execution
    pub ast_node_renames: FxHashMap<(ModuleId, TextRange), String>,
}
```

### ExecutionStep (Minimal Instructions)

```rust
pub enum ExecutionStep {
    /// Insert a pre-built AST statement
    InsertStatement { stmt: Stmt },

    /// Copy a statement from source, applying renames
    CopyStatement {
        source_module: ModuleId,
        item_id: ItemId,
    },
}
```

### ImportClassification (Semantic Categories)

```rust
pub enum ImportClassification {
    /// Hoist the import to bundle top (ONLY safe stdlib imports)
    /// Third-party imports are NEVER hoisted due to side effects
    Hoist { import_type: HoistType },

    /// Inline the imported symbols directly
    Inline {
        module_id: ModuleId,
        symbols: Vec<SymbolImport>,
    },

    /// Emulate the imported module as a namespace object
    EmulateAsNamespace { module_id: ModuleId, alias: String },
}
```

## BundleCompiler Responsibilities

The compiler encapsulates all the intelligence of the bundling process:

### 1. Initialization Phase

```rust
impl<'a> BundleCompiler<'a> {
    pub fn new(...) -> Result<Self> {
        // Initialize from analysis results
        let mut compiler = Self { /* fields */ };
        compiler.initialize_from_analysis();
        Ok(compiler)
    }

    fn initialize_from_analysis(&mut self) {
        // Extract symbol renames from conflict analysis
        self.add_symbol_renames(&self.analysis_results.symbol_conflicts);

        // Extract live items from tree-shaking
        self.add_tree_shake_decisions(&self.analysis_results.tree_shake_results);

        // Build module alias map
        self.populate_module_aliases();

        // Classify all imports
        self.classify_imports();

        // Classify modules (side effects, circular deps)
        self.classify_modules();
    }
}
```

### 2. Compilation Phase

```rust
pub fn compile(self) -> Result<BundleProgram> {
    let mut steps = Vec::new();

    // Phase 1: Compile hoisted imports
    let hoisted_steps = self.compile_hoisted_imports()?;
    steps.extend(hoisted_steps);

    // Phase 2: Compile namespace infrastructure
    let namespace_steps = self.compile_namespace_modules()?;
    steps.extend(namespace_steps);

    // Phase 3: Compile entry module body
    let entry_steps = self.compile_entry_module()?;
    steps.extend(entry_steps);

    // Generate AST node renames from symbol renames
    let ast_node_renames = self.generate_ast_node_renames();

    Ok(BundleProgram {
        steps,
        ast_node_renames,
    })
}
```

### 3. Import Hoisting Logic

**CRITICAL**: Import hoisting rules:

1. Only `__future__` and stdlib imports can be safely hoisted (no side effects)
2. Third-party imports are NEVER hoisted due to potential side effects
3. Only LIVE imports (marked as used in the dependency graph) are included
4. Unused imports are automatically excluded based on graph analysis

**Note**: The tree-shaking analysis must properly exclude unused imports even in otherwise used modules. The `is_import_required` method should be used to determine if an import is actually needed.

**FINDING**: There's a sequencing issue with namespace population and symbol renaming:

1. When symbols are renamed to avoid conflicts (e.g., `message` → `message_conflict_module`), the renamed definitions are copied via CopyStatement
2. The namespace assignments generated by InsertStatement try to reference the original names
3. This causes runtime errors because the original names don't exist - only the renamed versions

**TODO**: The namespace population logic needs to be aware of symbol renames and use the renamed symbol names when creating namespace assignments.

```rust
fn compile_hoisted_imports(&self) -> Result<Vec<ExecutionStep>> {
    let mut steps = Vec::new();
    let mut imported_modules = HashSet::new();

    // Categorize imports - ONLY stdlib and __future__
    let mut future_imports = Vec::new();
    let mut stdlib_imports = Vec::new();

    for ((module_id, item_id), classification) in &self.classified_imports {
        // Only process LIVE imports (skip unused imports)
        if !is_import_live(module_id, item_id) {
            continue;
        }

        if let ImportClassification::Hoist { import_type } = classification {
            // Build AST and check if safe to hoist
            let stmt = build_import_ast(import_type);
            if module_name == "__future__" {
                future_imports.push(stmt);
            } else if is_stdlib_module(module_name) {
                stdlib_imports.push(stmt);
            }
            // Third-party imports are NOT hoisted
        }
    }

    // Sort for determinism
    sort_import_statements(&mut stdlib_imports);

    // Build execution steps in order: __future__ then stdlib only
    for stmt in future_imports.into_iter().chain(stdlib_imports) {
        steps.push(ExecutionStep::InsertStatement { stmt });
    }

    Ok(steps)
}
```

### 4. Namespace Module Generation

```rust
fn compile_namespace_modules(&self) -> Result<Vec<ExecutionStep>> {
    let mut steps = Vec::new();

    // Add types import if needed
    steps.push(ExecutionStep::InsertStatement {
        stmt: ast_builder::import("types"),
    });

    for (module_id, namespace_name) in &namespace_modules {
        // Copy module body (excluding imports)
        for item_id in self.live_items.get(module_id) {
            if !is_import(item_id) {
                steps.push(ExecutionStep::CopyStatement {
                    source_module: *module_id,
                    item_id: *item_id,
                });
            }
        }

        // Create namespace object
        steps.push(ExecutionStep::InsertStatement {
            stmt: ast_builder::assign(namespace_name, ast_builder::call("types.SimpleNamespace")),
        });

        // Populate namespace attributes
        for symbol in get_public_symbols(module_id) {
            steps.push(ExecutionStep::InsertStatement {
                stmt: ast_builder::assign_attribute(
                    namespace_name,
                    &symbol,
                    ast_builder::name(&symbol),
                ),
            });
        }
    }

    Ok(steps)
}
```

## Bundle VM Implementation

The VM is now a pure mechanical executor with zero decision-making:

```rust
// bundle_vm.rs (renamed from plan_executor.rs)

pub fn run(program: &BundleProgram, context: &ExecutionContext) -> Result<ModModule> {
    let mut final_body = Vec::new();

    for step in &program.steps {
        match step {
            ExecutionStep::InsertStatement { stmt } => {
                // Direct insertion of pre-built AST
                final_body.push(stmt.clone());
            }

            ExecutionStep::CopyStatement {
                source_module,
                item_id,
            } => {
                // Get statement from source
                let stmt = get_statement(&context.source_asts, *source_module, *item_id)?;

                // Apply renames mechanically
                let renamed_stmt =
                    apply_ast_renames(stmt, &program.ast_node_renames, *source_module);

                final_body.push(renamed_stmt);
            }
        }
    }

    Ok(ModModule {
        body: final_body,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}
```

### Key Properties of the VM

1. **No State**: The VM maintains no internal state between steps
2. **No Logic**: All decisions were made during compilation
3. **Pure Functions**: Each operation is deterministic and side-effect free
4. **Simple Interface**: Takes BundleProgram and ExecutionContext, returns AST

## AST Builder Module

To ensure deterministic AST generation:

```rust
// ast_builder.rs
use ruff_python_ast::*;
use ruff_text_size::TextRange;

/// All synthetic nodes use default ranges
const SYNTHETIC_RANGE: TextRange = TextRange::default();

pub fn import(module_name: &str) -> Stmt {
    Stmt::Import(StmtImport {
        names: vec![Alias {
            name: Identifier::new(module_name, SYNTHETIC_RANGE),
            asname: None,
            range: SYNTHETIC_RANGE,
        }],
        range: SYNTHETIC_RANGE,
    })
}

pub fn assign(target: &str, value: Expr) -> Stmt {
    Stmt::Assign(StmtAssign {
        targets: vec![Expr::Name(ExprName {
            id: Name::new(target),
            ctx: ExprContext::Store,
            range: SYNTHETIC_RANGE,
        })],
        value: Box::new(value),
        range: SYNTHETIC_RANGE,
    })
}

// ... more factory functions
```

## AST Node Rename Generation

The compiler generates precise AST node renames using semantic information:

```rust
fn generate_ast_node_renames(&self) -> FxHashMap<(ModuleId, TextRange), String> {
    let mut ast_node_renames = FxHashMap::default();

    let Some(semantic_provider) = self.semantic_provider else {
        return ast_node_renames;
    };

    for (global_binding_id, new_name) in &self.symbol_renames {
        let module_id = global_binding_id.module_id;
        let binding_id = global_binding_id.binding_id;

        if let Some(Ok(semantic_model)) = semantic_provider.get_model(module_id) {
            let binding = semantic_model.binding(binding_id);

            // Add definition
            ast_node_renames.insert((module_id, binding.range), new_name.clone());

            // Add all references
            for reference_id in &binding.references {
                let reference = semantic_model.reference(*reference_id);
                ast_node_renames.insert((module_id, reference.range()), new_name.clone());
            }
        }
    }

    ast_node_renames
}
```

## Benefits of This Architecture

1. **Clean Separation**: Compiler holds all state, VM is stateless
2. **Single Responsibility**: Each component does exactly one thing
3. **Testability**: BundleProgram can be inspected and validated independently
4. **Maintainability**: Changes to bundling logic never affect the VM
5. **Performance**: Simple VM enables easy optimization
6. **Debuggability**: BundleProgram is a complete, inspectable representation

## Implementation Status

✅ **Completed**:

- BundleCompiler extracted from BundlePlan
- BundleProgram as clean interface between compiler and VM
- Renamed plan_executor.rs to bundle_vm.rs
- Renamed execute_plan to run
- ImportClassification variants renamed for clarity
- AST node rename generation implemented
- All namespace generation moved to compiler
- Fixed unused import removal: `classify_imports` now skips imports not in `live_items`

## Key Design Decisions

1. **Why BundleCompiler + BundleProgram?**
   - Enforces separation between compilation state and execution
   - Makes it impossible for execution to depend on compilation internals
   - Enables future optimizations on BundleProgram without touching compiler

2. **Why rename to bundle_vm?**
   - Better reflects the architectural metaphor
   - Makes the "dumb executor" nature explicit
   - Aligns with industry-standard compiler terminology

3. **Why semantic import names?**
   - `Hoist` clearly indicates moving to top
   - `Inline` clearly indicates direct insertion
   - `EmulateAsNamespace` clearly indicates namespace object creation

## Future Considerations

1. **BundleProgram Optimization**: Could add optimization passes between compilation and execution
2. **Serialization**: BundleProgram could be serialized for caching or debugging
3. **Parallel Execution**: Simple instructions enable parallel processing where safe
