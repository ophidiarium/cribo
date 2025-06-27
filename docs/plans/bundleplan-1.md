Excellent! The deep-code-reasoning review has provided clear, actionable recommendations. Here's a summary of the key
architectural insights and required changes:

Key Findings

1. Architectural Mismatch: The implementation doesn't fully follow the "Compiler → Bytecode → VM" model from the design doc.
   The executor still contains logic that should be in the plan builder.
2. Naming Clarity Issues: Current names don't clearly convey their architectural role:
   - plan_executor → should be bundle_vm
   - ImportClassification variants need semantic clarity
   - BundlePlan is doing too much - needs splitting
3. Single Responsibility Violation: BundlePlan contains both compilation state and final output.

Immediate Actions Required

1. Rename ImportClassification Variants

enum ImportClassification {
/// Hoist the import to the top of the bundle (stdlib/third-party)
Hoist { import_type: HoistType },

      /// Inline the imported symbols directly into bundle scope
      /// e.g., `from .utils import helper` → `def helper(): ...`
      Inline { module_id: ModuleId, symbols: Vec<SymbolImport> },

      /// Emulate the module as a namespace object
      /// e.g., `import .utils` → `utils = SimpleNamespace()`
      EmulateAsNamespace { module_id: ModuleId, alias: String },

}

2. Extract BundleCompiler from BundlePlan

- Create BundleCompiler struct to hold all compilation state
- Create BundleProgram struct as the clean output (just Vec<ExecutionStep> + rename map)
- Move all logic from build_execution_plan into BundleCompiler::compile()

3. Refactor Plan Executor to Bundle VM

- Rename plan_executor.rs → bundle_vm.rs
- Remove ALL logic (namespace generation, etc.) from the VM
- Make it a pure instruction interpreter

4. Move All AST Generation to Compiler

- The compiler should generate ALL AST nodes upfront
- Use ast_builder primitives to create complete statements
- Package everything as ExecutionStep::InsertStatement

5. Keep ast_builder Primitive

- No domain-specific methods like create_namespace_object
- Keep it as a pure AST factory
- All semantic composition happens in the compiler

The Two-Variant ExecutionStep is Perfect

The review confirmed that our minimal design with just InsertStatement and CopyStatement is ideal - it enforces the
architectural boundary perfectly.

Next Steps

1. I should start by renaming the ImportClassification variants
2. Then extract BundleCompiler from BundlePlan
3. Move all namespace generation from executor to compiler
4. Finally rename plan_executor to bundle_vm

This will achieve the true "dumb executor" vision from the design document.
