# Bundling Plan Execution Architecture

This document describes the end-to-end architecture of Cribo's bundling system, focusing on the separation between analysis (decision-making) and execution (mechanical transformation).

## Overview

Cribo uses a two-phase architecture that separates bundling decisions from their execution:

1. **Analysis Phase**: Analyzes the Python codebase and makes all bundling decisions
2. **Execution Phase**: Mechanically applies those decisions to generate the bundled output

This separation ensures deterministic, testable bundling with clear data flow.

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           ANALYSIS PHASE                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  Source Files          Module Discovery        Dependency Graph             │
│  ┌─────────┐          ┌──────────────┐       ┌────────────────┐          │
│  │ main.py │ ───────> │   Resolver   │ ───>  │  CriboGraph    │          │
│  │ utils.py│          │              │       │                │          │
│  │ lib.py  │          └──────────────┘       │ - ModuleDepGraph│          │
│  └─────────┘                │                │ - ItemData      │          │
│       │                     │                │ - Dependencies  │          │
│       │              Module Registry          └────────────────┘          │
│       │              ┌──────────────┐                │                    │
│       └────────────> │   Modules    │                │                    │
│                      │   Metadata   │                │                    │
│                      └──────────────┘                │                    │
│                                                      ▼                    │
│  ┌─────────────────────────────────────────────────────────────────┐     │
│  │                    Analysis Passes                               │     │
│  ├─────────────────────────────────────────────────────────────────┤     │
│  │                                                                  │     │
│  │  ┌──────────────┐  ┌──────────────┐  ┌───────────────────┐    │     │
│  │  │  Circular    │  │   Symbol     │  │   Tree-Shaking    │    │     │
│  │  │ Dependencies │  │  Conflicts   │  │    Analysis       │    │     │
│  │  └──────────────┘  └──────────────┘  └───────────────────┘    │     │
│  │         │                  │                    │               │     │
│  │         ▼                  ▼                    ▼               │     │
│  │  ┌──────────────┐  ┌──────────────┐  ┌───────────────────┐    │     │
│  │  │   Import     │  │   Symbol     │  │   Live Items      │    │     │
│  │  │  Rewrites    │  │   Renames    │  │   Selection       │    │     │
│  │  └──────────────┘  └──────────────┘  └───────────────────┘    │     │
│  │                                                                  │     │
│  └─────────────────────────────────────────────────────────────────┘     │
│                                   │                                        │
│                                   ▼                                        │
│                          ┌─────────────────┐                              │
│                          │   BundlePlan    │                              │
│                          ├─────────────────┤                              │
│                          │ • execution_plan│                              │
│                          │ • live_items    │                              │
│                          │ • symbol_renames│                              │
│                          │ • import_rewrites│                              │
│                          │ • metadata      │                              │
│                          └─────────────────┘                              │
│                                   │                                        │
└───────────────────────────────────┼────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           EXECUTION PHASE                                    │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  BundlePlan              Plan Executor           Python AST                │
│  ┌─────────────┐        ┌─────────────┐        ┌─────────────┐           │
│  │ExecutionStep│ ─────> │  Stateless  │ ─────> │  Generated  │           │
│  │  - Hoist    │        │  Executor   │        │   Module    │           │
│  │  - Inline   │        │             │        │             │           │
│  │  - Wrap     │        └─────────────┘        └─────────────┘           │
│  └─────────────┘               │                      │                    │
│                                │                      │                    │
│                         Source ASTs            AST Transformer             │
│                         ┌──────────┐           ┌──────────────┐           │
│                         │ Module   │           │   Rename      │           │
│                         │ ASTs     │ ────────> │   Symbols     │           │
│                         └──────────┘           └──────────────┘           │
│                                                        │                    │
│                                                        ▼                    │
│                                                 ┌─────────────┐            │
│                                                 │   Output    │            │
│                                                 │  bundle.py  │            │
│                                                 └─────────────┘            │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Component Details

### Analysis Phase Components

#### 1. Module Discovery & Resolution

- **Resolver**: Discovers Python modules and resolves import paths
- **Module Registry**: Maintains metadata about all discovered modules
- **Classification**: Determines if imports are stdlib, third-party, or first-party

#### 2. Dependency Graph (CriboGraph)

- **ModuleDepGraph**: Fine-grained dependency tracking per module
- **ItemData**: Metadata for each statement/item in a module
  - `statement_index`: Position in original source file
  - `var_decls`, `read_vars`: Variable usage tracking
  - `has_side_effects`: Whether statement has side effects
- **Dependencies**: Item-to-item dependency relationships

#### 3. Analysis Passes

Each pass examines the graph and produces specific decisions:

- **Circular Dependencies**: Detects import cycles and suggests resolutions
- **Symbol Conflicts**: Identifies naming conflicts between modules
- **Tree-Shaking**: Determines which code is actually used

#### 4. BundlePlan

The central data structure containing all bundling decisions:

```rust
pub struct BundlePlan {
    // Primary execution driver
    pub execution_plan: Vec<ExecutionStep>,

    // Live code tracking
    pub live_items: FxHashMap<ModuleId, Vec<ItemId>>,

    // Symbol renaming decisions
    pub symbol_renames: IndexMap<GlobalBindingId, String>,
    pub ast_node_renames: FxHashMap<(ModuleId, TextRange), String>,

    // Import handling
    pub import_rewrites: Vec<ImportRewrite>,
    pub hoisted_imports: Vec<HoistedImport>,

    // Module metadata
    pub module_metadata: FxHashMap<ModuleId, ModuleMetadata>,
}
```

### Execution Phase Components

#### 1. ExecutionStep Enum

Granular operations that the executor performs:

```rust
pub enum ExecutionStep {
    // Hoist future imports to top
    HoistFutureImport {
        name: String,
    },

    // Hoist stdlib imports
    HoistStdlibImport {
        name: String,
    },

    // Define init function for wrapped modules
    DefineInitFunction {
        module_id: ModuleId,
    },

    // Call init function to instantiate module
    CallInitFunction {
        module_id: ModuleId,
        target_variable: String,
    },

    // Inline a statement from source
    InlineStatement {
        module_id: ModuleId,
        item_id: ItemId,
    },
}
```

#### 2. Plan Executor

A stateless executor that:

- Takes BundlePlan and source ASTs as input
- Processes ExecutionSteps sequentially
- Performs no analysis or decision-making
- Applies AST transformations mechanically

#### 3. AST Transformations

- **Symbol Renaming**: Applies renames from `ast_node_renames`
- **Import Rewriting**: Modifies import statements per `import_rewrites`
- **Statement Retrieval**: Uses `statement_index` to fetch correct statements

## Data Flow

### 1. Analysis Flow

```
Source Files → Parser → AST → Graph Builder → CriboGraph → Analyzers → BundlePlan
```

### 2. Execution Flow

```
BundlePlan + Source ASTs → Plan Executor → Transformed AST → Code Generator → Output
```

### 3. Statement Tracking

```
Source Statement → ItemId + statement_index → ExecutionStep → Retrieved Statement
```

## Key Design Principles

### 1. Separation of Concerns

- **Analysis**: All intelligence and decision-making
- **Execution**: Pure mechanical transformation
- **No backflow**: Execution never influences analysis

### 2. Deterministic Output

- Sorted execution plans ensure consistent ordering
- All non-deterministic operations happen in analysis
- Execution is purely functional transformation

### 3. Testability

- BundlePlan can be serialized and inspected
- Execution can be tested with mock plans
- Each component has clear inputs/outputs

### 4. Incremental Development

- New ExecutionStep variants can be added
- Analyzers can be developed independently
- Executor remains simple and stable

## Current Implementation Status

### ✅ Implemented

- Basic ExecutionStep enum with InlineStatement
- Plan executor framework
- Statement index tracking
- Fallback for missing tree-shaking
- Statement ordering preservation

### 🚧 In Progress

- AST symbol renaming
- Import rewriting
- Hoisted import generation

### 📋 TODO

- Wrapped module support (DefineInitFunction, CallInitFunction)
- Advanced tree-shaking integration
- Circular dependency resolution
- Full import merging

## Example: Simple Bundle

For a simple Python file:

```python
x = 5
y = 10
z = x + y
print(z)
```

The execution plan would be:

```
1. InlineStatement { module_id: 0, item_id: 0 }  # x = 5
2. InlineStatement { module_id: 0, item_id: 1 }  # y = 10  
3. InlineStatement { module_id: 0, item_id: 2 }  # z = x + y
4. InlineStatement { module_id: 0, item_id: 3 }  # print(z)
```

Each ItemId maps to its statement via `statement_index`, ensuring correct ordering.
