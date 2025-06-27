# Immediate Fix Plan: Stdlib Import Alias Issue

## Problem

The bundle compiler generates hoisted imports using stale metadata from the graph, which was built before stdlib normalization removed aliases. This causes imports like `import json as j` to appear in the bundle when they should be `import json`.

## Root Cause

1. Graph is built with original imports (includes aliases)
2. Stdlib normalization transforms AST (removes aliases)
3. Bundle compiler uses graph metadata to generate hoisted imports
4. Result: Hoisted imports have aliases that no longer exist in the code

## Immediate Fix Options

### Option A: Update Graph After Normalization

**Risk:** Low
**Approach:**

1. After stdlib normalization, update the graph's import metadata
2. Remove alias information for normalized imports
3. Bundle compiler will then generate correct imports

### Option B: Check Normalization State in Compiler

**Risk:** Medium
**Approach:**

1. Add a flag to track which modules have been normalized
2. Bundle compiler checks this flag when generating imports
3. Strip aliases for normalized modules

### Option C: Apply Normalization Info During Hoisting

**Risk:** Low
**Approach:**

1. Store normalization results in AnalysisResults
2. Bundle compiler consults this when generating hoisted imports
3. Generate canonical imports for normalized modules

## Recommended Immediate Fix: Option C

This aligns best with the future architecture while solving the immediate problem.

### Implementation Steps

1. **Capture normalization results**
   ```rust
   // In AnalysisResults
   pub stdlib_normalizations: FxHashMap<ModuleId, NormalizationResult>,
   ```

2. **Store during normalization**
   ```rust
   // In orchestrator after normalization
   analysis_results.stdlib_normalizations.insert(
       module_id, 
       normalization_result
   );
   ```

3. **Use in bundle compiler**
   ```rust
   // When generating hoisted import
   if let Some(norm) = analysis_results.stdlib_normalizations.get(&module_id) {
       // Generate import without alias
       generate_canonical_import(module_name)
   } else {
       // Use original import with alias
       generate_import_with_alias(module_name, alias)
   }
   ```

## Testing

1. Verify `xfail_alias_transformation_test` passes
2. Check all stdlib imports in snapshots are canonical
3. Ensure no functional regressions

## Long-term Solution

This immediate fix is a stepping stone to the full Transformation Plan architecture. The normalization results stored here will eventually become part of the TransformationMetadata system.

## Implementation Notes

This fix addresses the immediate issue while moving us toward the correct architecture. It serves as a stepping stone to the full Transformation Plan system.
