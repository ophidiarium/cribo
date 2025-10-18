# Question: Rust Borrow Checker Constraints in BundleOrchestrator Implementation

**To**: Original architect of `refactor-bundle-modules-decomposition.md`
**From**: Implementation team
**Date**: 2025-10-18
**Context**: PR #395 - Bundle Modules Decomposition

## Summary

We've successfully extracted all 6 phases as proposed, but hit Rust borrow checker constraints when wiring them together in the `BundleOrchestrator`. The phases work perfectly in isolation (all 170 tests pass), but the orchestrator can't be implemented as originally proposed due to lifetime and borrowing conflicts.

**Current Status**:

- ✅ All phases extracted with tests (InitializationPhase, ClassificationPhase, ProcessingPhase, EntryModulePhase, PostProcessingPhase)
- ❌ BundleOrchestrator cannot be implemented as proposed - currently just delegates back to original `bundle_modules()`
- ❌ Result: ~2000 lines of dead code with poor coverage (20%)

**We need architectural guidance** on how to resolve the borrow checker constraints to actually use the extracted phases.

---

## The Architectural Issue

### Proposed Design (from the proposal document)

```rust
pub struct BundleOrchestrator<'a> {
    bundler: &'a mut Bundler<'a>,
    context: BundleContext<'a>,
}

impl<'a> BundleOrchestrator<'a> {
    pub fn bundle(&mut self, params: &BundleParams<'a>) -> ModModule {
        let init_result = self.initialization_phase(params)?;
        let preprocess_result = self.preprocessing_phase(&init_result)?;
        let analysis_result = self.analysis_phase(&preprocess_result)?;
        let process_result = self.processing_phase(&analysis_result)?;
        let postprocess_result = self.postprocessing_phase(&process_result)?;
        self.finalization_phase(postprocess_result)
    }
}
```

### What We Implemented (following the proposal)

Each phase holds a mutable reference to the bundler:

```rust
pub struct InitializationPhase<'a> {
    bundler: &'a mut Bundler<'a>,
}

impl<'a> InitializationPhase<'a> {
    pub fn execute(&mut self, params: &BundleParams<'a>) -> InitializationResult {
        // Uses self.bundler to access and mutate bundler state
    }
}
```

### The Borrow Checker Problem

When we try to implement the orchestrator as proposed:

```rust
pub fn bundle<'a>(bundler: &'a mut Bundler<'a>, params: &BundleParams<'a>) -> ModModule {
    // Phase 1: Initialization
    let mut init_phase = InitializationPhase::new(bundler); // First mutable borrow
    let init_result = init_phase.execute(params);

    // Phase 2: Classification
    let mut classification_phase = ClassificationPhase::new(bundler); // ERROR! Second mutable borrow
    let classification = classification_phase.execute(&modules, params.python_version);

    // ... more phases
}
```

**Compiler Error**:

```
error[E0499]: cannot borrow `*bundler` as mutable more than once at a time
  |
  | let mut init_phase = InitializationPhase::new(bundler);
  |                                                ------- first mutable borrow occurs here
  |                                                argument requires that `*bundler` is borrowed for `'a`
  | let mut classification_phase = ClassificationPhase::new(bundler);
  |                                                          ^^^^^^^ second mutable borrow occurs here
```

### Why This Happens

The problem is that `InitializationPhase` holds `&'a mut Bundler<'a>`, which borrows `bundler` for the lifetime `'a`. When we try to create the next phase, we need another mutable borrow of `bundler`, but Rust won't allow it because the first borrow is still active (the `init_phase` variable is still in scope).

**We tried dropping the phase immediately**:

```rust
let init_result = {
    let mut init_phase = InitializationPhase::new(bundler);
    init_phase.execute(params)  // Borrow ends here
};

let classification = {
    let mut classification_phase = ClassificationPhase::new(bundler);  // Still fails!
    classification_phase.execute(&modules, params.python_version)
};
```

But this still fails because `BundleParams<'a>` and other parameters capture the lifetime `'a`, preventing the borrow from ending.

---

## Attempted Solutions & Why They Failed

### Attempt 1: Scoped Drops (Failed)

**Tried**: Using blocks to drop phases immediately
**Failed**: Lifetimes in return types and parameters keep the borrow active

### Attempt 2: Non-Lexical Lifetimes (Failed)

**Tried**: Relying on NLL to end borrows early
**Failed**: Lifetime `'a` in signatures propagates through all phase interactions

### Attempt 3: Stateless Phases (Not Attempted Yet)

**Idea**: Make phases not hold `&mut Bundler<'a>` but instead take it as method parameter:

```rust
pub struct InitializationPhase; // No field

impl InitializationPhase {
    pub fn execute(
        &self,
        bundler: &mut Bundler<'_>,
        params: &BundleParams<'_>,
    ) -> InitializationResult {
        // ...
    }
}
```

**Question**: Is this the intended approach? Does it break encapsulation?

### Attempt 4: Split Bundler State (Not Attempted Yet)

**Idea**: Extract mutable state from `Bundler` into a separate `BundlerState` struct that phases can own/mutate:

```rust
pub struct BundlerState {
    module_synthetic_names: FxIndexMap<ModuleId, String>,
    inlined_modules: FxIndexSet<ModuleId>,
    // ... other mutable fields
}

pub struct InitializationPhase {
    // No bundler reference
}

impl InitializationPhase {
    pub fn execute(
        &self,
        state: &mut BundlerState,
        resolver: &Resolver,
        ...
    ) -> InitializationResult {
        // ...
    }
}
```

**Question**: Is this the right direction?

---

## Specific Questions for the Architect

### Q1: Lifetime Design Intent

In the proposal, you show phases holding `&'a mut Bundler<'a>`. How did you envision the orchestrator creating multiple phases sequentially without violating Rust's borrow rules?

Were you expecting:

- A) Phases to be stateless (no bundler field)?
- B) Some form of interior mutability (`RefCell`, `Cell`)?
- C) Bundler state to be extracted into a separate owned struct?
- D) Something else entirely?

### Q2: Phase Ownership Model

The proposal shows:

```rust
pub struct BundleOrchestrator<'a> {
    bundler: &'a mut Bundler<'a>,
    context: BundleContext<'a>,
}
```

Should the orchestrator:

- **Own** the bundler temporarily during bundling?
- **Borrow** it mutably throughout the entire process?
- **Split** bundler into immutable (resolver, graph) and mutable (state) parts?

### Q3: Current Implementation Assessment

Given the borrow checker constraints, what's the best path forward?

**Option A**: Refactor phases to be stateless

```rust
pub struct InitializationPhase;

impl InitializationPhase {
    pub fn execute(bundler: &mut Bundler<'_>, params: &BundleParams<'_>) -> InitializationResult {
        // Pass bundler as parameter instead of holding it
    }
}
```

**Option B**: Extract mutable state from Bundler

```rust
pub struct BundlingContext {
    // All mutable state from Bundler
    module_synthetic_names: FxIndexMap<ModuleId, String>,
    inlined_modules: FxIndexSet<ModuleId>,
    // ... etc
}

pub struct InitializationPhase;

impl InitializationPhase {
    pub fn execute(
        context: &mut BundlingContext,
        resolver: &Resolver,
        params: &BundleParams<'_>,
    ) -> InitializationResult {
        // Phases mutate context instead of bundler
    }
}
```

**Option C**: Use interior mutability patterns

```rust
pub struct Bundler<'a> {
    state: RefCell<BundlerState>, // Interior mutability
    resolver: &'a Resolver,
    // ...
}
```

**Option D**: Something else you envisioned?

### Q4: Trade-offs You Considered

When designing this architecture, what trade-offs did you consider between:

- Code organization clarity vs. Rust ownership complexity?
- Testability vs. production integration difficulty?
- Incremental refactoring vs. complete rewrite?

---

## Current Code Statistics

**What We Have**:

- 6 phase modules: ~2010 lines of code
- 22 new unit tests (all passing)
- 170 total tests (all passing)
- **Code coverage: 20%** (phases never execute in production)
- **bundle_modules: Still 1,330 lines** (unchanged)

**What We Need**:

- Phases actually called from `bundle_modules` or orchestrator
- Code coverage: >80% for phase code
- `bundle_modules` reduced to ~200 lines (just orchestration)
- Production bundles using the new phase-based code

---

## Example of the Borrow Checker Issue

Here's the specific code that fails:

```rust
// File: crates/cribo/src/code_generator/phases/orchestrator.rs (attempted implementation)

pub fn bundle<'a>(bundler: &'a mut Bundler<'a>, params: &BundleParams<'a>) -> ModModule {
    let mut final_body = Vec::new();

    // Phase 1 - This works
    let mut init_phase = InitializationPhase::new(bundler); // Borrows bundler for 'a
    let init_result = init_phase.execute(params); // Returns InitializationResult

    // At this point, init_phase still holds the borrow of bundler for lifetime 'a
    // because InitializationResult contains references tied to 'a (via params)

    // Phase 2 - This FAILS
    let mut classification_phase = ClassificationPhase::new(bundler);
    // ERROR[E0499]: cannot borrow `*bundler` as mutable more than once at a time
    //               first borrow from init_phase.execute(params) is still active
    //               because params has lifetime 'a and we returned InitializationResult
}
```

**Full compiler output**:

```
error[E0499]: cannot borrow `*bundler` as mutable more than once at a time
  --> crates/cribo/src/code_generator/phases/orchestrator.rs:67:65
   |
45 |     pub fn bundle<'a>(bundler: &'a mut Bundler<'a>, params: &BundleParams<'a>) -> ModModule {
   |                       -- lifetime `'a` defined here
...
49 |         let init_result = InitializationPhase::new(bundler).execute(params);
   |                           ---------------------------------
   |                           |                        |
   |                           |                        first mutable borrow occurs here
   |                           argument requires that `*bundler` is borrowed for `'a`
...
67 |         let mut symbol_renames = bundler.collect_symbol_renames(&modules, &semantic_ctx);
   |                                  ^^^^^^^ second mutable borrow occurs here
```

---

## What Would Success Look Like?

**Ideal End State** (what we thought we were building):

1. `bundle_modules()` becomes ~200 lines of orchestration:

```rust
pub fn bundle_modules(&mut self, params: &BundleParams<'a>) -> ModModule {
    let orchestrator = BundleOrchestrator::new(self);
    orchestrator.bundle(params)
}
```

2. Each phase actually runs in production (high code coverage)

3. Phases can be tested independently with real bundler state

4. Future features can be added by creating new phases or extending existing ones

**Current Reality**:

```rust
pub fn bundle_modules(&mut self, params: &BundleParams<'a>) -> ModModule {
    // Still 1,330 lines of original code
    // All the extracted phase code never executes
}
```

---

## Request for Guidance

**What architectural pattern did you envision** for resolving these Rust ownership constraints?

Please provide:

1. **Concrete code examples** showing how phases should be structured to work with the borrow checker
2. **Bundler refactoring guidance** if the current Bundler design needs changes
3. **Alternative architecture** if the current approach is fundamentally incompatible with Rust's ownership rules
4. **Trade-off analysis** of different approaches (stateless vs. state extraction vs. interior mutability)

We're committed to delivering a working solution that actually improves the codebase, not just adding dead code. Your guidance on the intended architectural pattern would be invaluable.

---

## References

- **PR**: https://github.com/ophidiarium/cribo/pull/395
- **Proposal**: `docs/proposals/refactor-bundle-modules-decomposition.md`
- **Implementation Branch**: `refactor/bundle-modules-decomposition`
- **Key Files**:
  - `crates/cribo/src/code_generator/phases/orchestrator.rs` - Failed orchestrator
  - `crates/cribo/src/code_generator/phases/*.rs` - Extracted phases (all working in isolation)
  - `crates/cribo/src/code_generator/bundler.rs` - Original bundle_modules (unchanged)
