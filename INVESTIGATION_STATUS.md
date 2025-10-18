# Bundle Modules Refactoring - Investigation Status

**Date**: 2025-10-19
**Token Usage**: 51%
**Status**: 168/170 tests passing, 2 failures under investigation

## What's Working

- ✅ All 6 phases extracted and stateless
- ✅ BundleOrchestrator fully wired
- ✅ bundle_modules delegates to orchestrator (5 lines)
- ✅ Phase code executes in production
- ✅ 168 tests passing

## Current Blockers

**2 Test Failures**:

- `ast_rewriting_globals_collision`
- `test_ecosystem_all` (depends on above)

## Investigation Findings

### Key Discoveries

1. **Absolute Path Module Names**: Module names like `/.Volumes.workspace.core.database` exist on BOTH main and my branch
2. **Both Generate Syntax Errors**: Both main and my branch generate `/.Volumes = __Volumes` (invalid Python)
3. **Tests Behave Differently**:
   - Main: Tests PASS despite syntax error
   - My branch: Tests FAIL on syntax error
4. **Not a Snapshot Issue**: Even with `INSTA_UPDATE=no`, main passes

### The Mystery

**Observation**: Main's generated code has syntax errors but tests pass. My branch's code has identical syntax errors but tests fail.

**Possible Explanations**:

1. Test execution environment difference
2. Caching in the test runner
3. Different code actually being executed (not what's generated to stdout)
4. The bundled file vs stdin execution difference
5. Some error recovery mechanism I'm not seeing

### Commits Analysis

- `307f3ff` (stateless complete): Tests PASS ✅
- `fb7ee7d` (orchestrator wired): Tests FAIL ❌
- `685ec80` (sanitize fix): Tests still FAIL ❌
- `HEAD` (5cbf44e): Tests still FAIL ❌

**Critical**: The bundle_modules_legacy function at HEAD generates the SAME broken output as my orchestrator, yet at 307f3ff (when it was still named `bundle_modules`), tests passed.

### Next Steps (for fresh debugging session)

1. **Understand test framework behavior**: Why does main pass with syntax errors?
2. **Check bundle_modules at 307f3ff**: Generate output and compare with current
3. **Examine state differences**: What bundler state is different between 307f3ff and fb7ee7d?
4. **Focus on**: The renaming from `bundle_modules` to `bundle_modules_legacy` might have changed how it's called or what state it has access to

### Technical Debt

- orchestrator.rs has latent empty src_dir bug (my fix in 3147f70 then reverted in 5cbf44e)
- namespace chain has parts[0] sanitization issue (my fix in 685ec80)
- Both fixes may be correct but addressing symptoms of deeper issue

## Recommended Approach

Rather than chasing症状, need to:

1. Bisect exactly which line of code in the orchestrator differs from original
2. Use diff on actual function bodies, not just outputs
3. Verify every phase method matches original bundle_modules logic exactly

The refactoring architecture is sound. Just need to ensure 100% logic fidelity.
