# File-Based Deduplication Implementation Proposal for Cribo

## Executive Summary

Currently, Cribo deduplicates modules by their import name only, not by file path. This means the same file can be bundled multiple times if imported under different names (e.g., `import utils` vs `from app import utils` vs `importlib.import_module("utils")`). This proposal outlines a comprehensive solution to implement proper file-based deduplication, ensuring each file appears only once in the bundle while maintaining correct behavior through Cribo's static inlining approach.

## Problem Statement

### Current Behavior

1. **Module deduplication by name only** (`cribo_graph.rs:719-723`):
   ```rust
   pub fn add_module(&mut self, name: String, path: PathBuf) -> ModuleId {
       if let Some(&id) = self.module_names.get(&name) {
           return id;  // Only checks name, not path!
       }
   ```

2. **Paths are not canonicalized** (`resolver.rs:498-501`):
   ```rust
   let package_init = current_path.join(part).join("__init__.py");
   if package_init.is_file() {
       return Ok(Some(package_init));  // Not canonicalized!
   }
   ```

3. **Import context is lost** (`orchestrator.rs:1297`):
   ```rust
   // Only passes module name, no context about where the import was found
   if let Ok(Some(import_path)) = params.resolver.resolve_module_path(import) {
   ```

4. **Static importlib calls not tracked** - Currently, `importlib.import_module("literal")` calls are not detected during the discovery phase, so these modules may not be bundled at all.

### Example Scenario

Given this structure:

```
project/
├── app/
│   └── utils.py  # def helper(): return "app"
├── lib/
│   └── utils.py  # def helper(): return "lib"  
├── shared/
│   └── common.py -> ../app/utils.py  # symlink
└── main.py
```

If `app/`, `lib/`, and `shared/` are in PYTHONPATH:

```python
# main.py
import utils as u1  # Could be app/utils.py or lib/utils.py
from app import utils as u2  # Definitely app/utils.py
from lib import utils as u3  # Definitely lib/utils.py
from shared import common  # Same file as app/utils.py via symlink

app_utils = importlib.import_module("app.utils")  # Also app/utils.py

# Current behavior: Each import creates a separate bundle entry
# Desired behavior: app/utils.py bundled once, with proper aliases for all names
```

## Proposed Solution

Since Cribo uses static inlining and eliminates runtime module systems entirely, the solution must:

1. Track ALL static imports (including `importlib.import_module()` with literal strings)
2. Canonicalize file paths to detect when different module names refer to the same file
3. Process each unique file only once
4. Create proper aliases for all the different ways the file can be imported
5. Rewrite all import forms to use the deduplicated symbols

### 1. Import Discovery Enhancements

#### A. Detect Static importlib Calls

Add importlib detection to the import discovery phase:

```rust
// In crate/cribo/src/visitors/import_discovery.rs

impl<'a> Visitor<'a> for ImportDiscoveryVisitor<'a> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Call(call) => {
                // Check for importlib.import_module("literal")
                if self.is_static_importlib_call(call) {
                    if let Some(module_name) = self.extract_literal_module_name(call) {
                        // Track this as a regular import
                        self.imports.push(DiscoveredImport {
                            module_name,
                            import_type: ImportType::ImportlibStatic,
                            location: call.range,
                        });
                    }
                }
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}

impl<'a> ImportDiscoveryVisitor<'a> {
    fn is_static_importlib_call(&self, call: &ExprCall) -> bool {
        match &*call.func {
            // importlib.import_module(...)
            Expr::Attribute(attr) => {
                if attr.attr.as_str() == "import_module" {
                    if let Expr::Name(name) = &*attr.value {
                        return name.id.as_str() == "importlib";
                    }
                }
            }
            // import_module(...) with prior "from importlib import import_module"
            Expr::Name(name) => {
                return name.id.as_str() == "import_module" && self.has_importlib_import();
            }
            _ => {}
        }
        false
    }

    fn extract_literal_module_name(&self, call: &ExprCall) -> Option<String> {
        // Only handle static string literals
        if let Some(Expr::StringLiteral(lit)) = call.arguments.args.first() {
            return Some(lit.value.to_str().to_string());
        }
        None
    }
}
```

### 2. Enhanced Data Structures

#### A. Comprehensive Module Tracking

Update `CriboGraph` to track all the ways a file can be imported:

```rust
// In crates/cribo/src/cribo_graph.rs

pub struct CriboGraph {
    // Existing fields
    modules: FxHashMap<ModuleId, ModuleDependencyGraph>,
    module_names: FxHashMap<String, ModuleId>,
    module_paths: FxHashMap<PathBuf, ModuleId>,

    // NEW: Track canonical paths
    module_canonical_paths: FxHashMap<ModuleId, PathBuf>,

    // NEW: Track all import names that resolve to each canonical file
    // This includes regular imports AND static importlib calls
    file_to_import_names: FxHashMap<PathBuf, FxIndexSet<String>>,

    // NEW: Track the primary module ID for each file
    // (The first import name discovered for this file)
    file_primary_module: FxHashMap<PathBuf, (String, ModuleId)>,
}
```

#### B. Import Type Tracking

Extend import types to include static importlib:

```rust
// In crates/cribo/src/cribo_graph.rs

#[derive(Debug, Clone, PartialEq)]
pub enum ImportType {
    /// import module
    Direct,
    /// from module import ...
    From,
    /// from . import ... (relative)
    Relative { level: usize },
    /// importlib.import_module("module") with static string
    ImportlibStatic,
}
```

### 3. Resolver Enhancements

#### A. Always Canonicalize Paths

```rust
// In crates/cribo/src/resolver.rs

impl Resolver {
    fn resolve_in_directory(
        &self,
        search_dir: &Path,
        descriptor: &ModuleDescriptor,
    ) -> Result<Option<PathBuf>> {
        // Try package directory first
        let package_dir = search_dir.join(&descriptor.path);
        let package_init = package_dir.join("__init__.py");
        if package_init.is_file() {
            debug!("Found package at: {package_init:?}");
            // ALWAYS canonicalize before returning
            return Ok(Some(self.canonicalize_path(package_init)?));
        }

        // Try module file
        let module_file = search_dir.join(format!("{}.py", descriptor.path));
        if module_file.is_file() {
            debug!("Found module at: {module_file:?}");
            return Ok(Some(self.canonicalize_path(module_file)?));
        }

        // Try namespace package (directory without __init__.py)
        if package_dir.is_dir() {
            debug!("Found namespace package at: {package_dir:?}");
            return Ok(Some(self.canonicalize_path(package_dir)?));
        }

        Ok(None)
    }

    fn canonicalize_path(&self, path: PathBuf) -> Result<PathBuf> {
        match path.canonicalize() {
            Ok(canonical) => Ok(canonical),
            Err(e) => {
                // Log warning but don't fail
                log::warn!("Failed to canonicalize {}: {}", path.display(), e);
                Ok(path)
            }
        }
    }
}
```

### 4. Dependency Graph Enhancements

#### A. File-Based Module Registration

Update module registration to handle multiple import names for the same file:

```rust
// In crates/cribo/src/cribo_graph.rs

impl CriboGraph {
    pub fn add_module(&mut self, name: String, path: PathBuf) -> ModuleId {
        // Always work with canonical paths
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());

        // Check if this exact import name already exists
        if let Some(&existing_id) = self.module_names.get(&name) {
            // Verify it's the same file
            if let Some(existing_canonical) = self.module_canonical_paths.get(&existing_id) {
                if existing_canonical == &canonical_path {
                    return existing_id; // Same import name, same file - reuse
                } else {
                    // Error: same import name but different files
                    // This shouldn't happen with proper PYTHONPATH management
                    log::error!(
                        "Import name '{}' refers to different files: {:?} and {:?}",
                        name,
                        existing_canonical,
                        canonical_path
                    );
                }
            }
        }

        // Track this import name for the file
        self.file_to_import_names
            .entry(canonical_path.clone())
            .or_default()
            .insert(name.clone());

        // Check if this file already has a primary module
        if let Some((primary_name, primary_id)) = self.file_primary_module.get(&canonical_path) {
            log::info!(
                "File {:?} already imported as '{}', adding additional import name '{}'",
                canonical_path,
                primary_name,
                name
            );

            // Create a new ModuleId that shares the same dependency graph
            // This allows different import names to have different dependency relationships
            // while still pointing to the same file
            let id = self.next_module_id();

            // Clone the dependency graph structure but with new module name
            let primary_graph = &self.modules[primary_id];
            let mut module = ModuleDependencyGraph::new(id, name.clone(), canonical_path.clone());

            // Share the same item registry (since it's the same file)
            module.share_items_with(primary_graph);

            self.modules.insert(id, module);
            self.module_names.insert(name, id);
            self.module_canonical_paths.insert(id, canonical_path);

            return id;
        }

        // This is the first time we're seeing this file
        let id = self.next_module_id();
        let module = ModuleDependencyGraph::new(id, name.clone(), canonical_path.clone());

        self.modules.insert(id, module);
        self.module_names.insert(name.clone(), id);
        self.module_paths.insert(canonical_path.clone(), id);
        self.module_canonical_paths
            .insert(id, canonical_path.clone());
        self.file_primary_module
            .insert(canonical_path, (name.clone(), id));

        log::debug!(
            "Registered module '{}' as primary for file {:?}",
            name,
            canonical_path
        );

        id
    }

    /// Get all import names that resolve to the same file as the given module
    pub fn get_file_import_names(&self, module_id: ModuleId) -> Vec<String> {
        if let Some(canonical_path) = self.module_canonical_paths.get(&module_id) {
            if let Some(names) = self.file_to_import_names.get(canonical_path) {
                return names.iter().cloned().collect();
            }
        }
        vec![]
    }

    /// Check if two modules refer to the same file
    pub fn same_file(&self, module_id1: ModuleId, module_id2: ModuleId) -> bool {
        if let (Some(path1), Some(path2)) = (
            self.module_canonical_paths.get(&module_id1),
            self.module_canonical_paths.get(&module_id2),
        ) {
            return path1 == path2;
        }
        false
    }
}
```

### 5. Code Generation Enhancements

#### A. Process Each File Only Once

The bundler must ensure each file's content is processed exactly once:

```rust
// In crates/cribo/src/code_generator.rs

impl Bundler {
    pub fn bundle_modules(&mut self, params: BundleParams<'_>) -> Result<ModModule> {
        // ... existing code ...

        // Group modules by canonical file path
        let file_groups = self.group_modules_by_file(params.modules);

        // Process each unique file only once
        let processed_files = self.process_unique_files(file_groups, params)?;

        // Generate the final bundle
        self.generate_bundle(processed_files, params)
    }

    fn group_modules_by_file(
        &self,
        modules: Vec<(String, ModModule, PathBuf, String)>,
    ) -> FxIndexMap<PathBuf, Vec<(String, ModModule, String)>> {
        let mut groups = FxIndexMap::default();

        for (name, ast, path, hash) in modules {
            let canonical = path.canonicalize().unwrap_or(path);
            groups
                .entry(canonical)
                .or_insert_with(Vec::new)
                .push((name, ast, hash));
        }

        // Log grouping results
        for (path, group) in &groups {
            if group.len() > 1 {
                let names: Vec<_> = group.iter().map(|(n, _, _)| n).collect();
                log::info!("File {:?} imported as: {:?}", path, names);
            }
        }

        groups
    }

    fn process_unique_files(
        &mut self,
        file_groups: FxIndexMap<PathBuf, Vec<(String, ModModule, String)>>,
        params: &BundleParams,
    ) -> Result<Vec<ProcessedFile>> {
        let mut processed = Vec::new();

        for (canonical_path, mut import_group) in file_groups {
            // Use the first import name as the primary
            // (In practice, we should use dependency order)
            let (primary_name, ast, hash) = import_group.remove(0);
            let additional_names: Vec<String> =
                import_group.into_iter().map(|(name, _, _)| name).collect();

            // Generate synthetic name for this file
            let synthetic_name = self.get_synthetic_module_name(&primary_name, &hash);

            // Register all import names to map to this synthetic module
            self.module_registry
                .insert(primary_name.clone(), synthetic_name.clone());
            for name in &additional_names {
                self.module_registry
                    .insert(name.clone(), synthetic_name.clone());
            }

            // Process the file content ONCE
            let processed_content = self.process_module_content(
                &primary_name,
                ast,
                &canonical_path,
                &synthetic_name,
                params,
            )?;

            processed.push(ProcessedFile {
                canonical_path,
                primary_name: primary_name.clone(),
                all_import_names: {
                    let mut names = vec![primary_name];
                    names.extend(additional_names);
                    names
                },
                synthetic_name,
                content: processed_content,
            });
        }

        Ok(processed)
    }
}
```

#### B. Import Rewriting

All import forms (regular imports and static importlib) must be rewritten identically:

```rust
// In crates/cribo/src/code_generator.rs

impl RecursiveImportTransformer {
    fn visit_stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Import(import) => self.transform_import(import),
            Stmt::ImportFrom(import_from) => self.transform_from_import(import_from),
            Stmt::Expr(expr_stmt) => {
                // Check for importlib.import_module() calls
                if let Some(new_expr) = self.try_transform_importlib(&expr_stmt.value) {
                    // Replace the entire statement with an assignment
                    *stmt = new_expr;
                }
            }
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }

    fn try_transform_importlib(&mut self, expr: &Expr) -> Option<Stmt> {
        if let Expr::Call(call) = expr {
            if self.is_importlib_import_module(call) {
                if let Some(Expr::StringLiteral(lit)) = call.arguments.args.first() {
                    let module_name = lit.value.to_str();

                    // Check if this module was bundled
                    if let Some(synthetic) = self.bundler.module_registry.get(module_name) {
                        // Transform: importlib.import_module("foo")
                        // Into: foo (just the reference to the inlined module)
                        // This works because we wrap modules that need to act like modules

                        // If this is part of an assignment, we handle it
                        // Otherwise, create a reference
                        return Some(Stmt::Expr(StmtExpr {
                            value: Box::new(Expr::Name(ExprName {
                                id: synthetic.into(),
                                ctx: ExprContext::Load,
                            })),
                        }));
                    }
                }
            }
        }
        None
    }

    fn transform_import(&mut self, import: &mut StmtImport) {
        // Existing logic for transforming regular imports
        // Works the same whether the module was discovered via
        // "import foo" or importlib.import_module("foo")
    }
}
```

#### C. Handle Import Assignments

For importlib calls that are assigned to variables:

```rust
impl RecursiveImportTransformer {
    fn visit_assign(&mut self, assign: &mut StmtAssign) {
        if assign.targets.len() == 1 {
            if let Expr::Call(call) = &*assign.value {
                if self.is_importlib_import_module(call) {
                    if let Some(Expr::StringLiteral(lit)) = call.arguments.args.first() {
                        let module_name = lit.value.to_str();

                        if let Some(synthetic) = self.bundler.module_registry.get(module_name) {
                            // Transform: foo = importlib.import_module("module")
                            // Into: foo = __cribo_wrapped_module
                            assign.value = Box::new(Expr::Name(ExprName {
                                id: synthetic.into(),
                                ctx: ExprContext::Load,
                            }));
                        }
                    }
                }
            }
        }
    }
}
```

### 6. Circular Dependency Handling

When the same file participates in circular dependencies under different import names:

```rust
// In crates/cribo/src/orchestrator.rs

impl Orchestrator {
    fn handle_circular_dependencies(&mut self, cycles: Vec<Vec<String>>) -> Result<()> {
        // Group cycle members by canonical file
        for cycle in cycles {
            let mut file_groups: FxHashMap<PathBuf, Vec<String>> = FxHashMap::default();

            for module_name in &cycle {
                if let Some(module) = self.graph.get_module_by_name(module_name) {
                    if let Some(canonical) = self.graph.get_canonical_path(module.module_id) {
                        file_groups
                            .entry(canonical.clone())
                            .or_default()
                            .push(module_name.clone());
                    }
                }
            }

            // Warn about same file appearing multiple times in cycle
            for (path, names) in file_groups {
                if names.len() > 1 {
                    log::warn!(
                        "Circular dependency contains same file {:?} imported as: {:?}",
                        path,
                        names
                    );
                    // The bundler will handle this by using the same wrapped module
                    // for all import names
                }
            }
        }

        Ok(())
    }
}
```

### 7. Edge Cases

#### A. Invalid Python Identifiers

For modules with names that aren't valid Python identifiers (imported via importlib):

```rust
// In crates/cribo/src/code_generator.rs

impl Bundler {
    fn get_synthetic_module_name(&self, module_name: &str, content_hash: &str) -> String {
        // Ensure the synthetic name is a valid Python identifier
        let safe_name = module_name
            .chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => c,
                _ => '_',
            })
            .collect::<String>();

        // Ensure it doesn't start with a digit
        let safe_name = if safe_name.chars().next().unwrap_or('_').is_numeric() {
            format!("m_{}", safe_name)
        } else {
            safe_name
        };

        format!("__cribo_{}_{}", &content_hash[..6], safe_name)
    }
}
```

#### B. Side Effects in Multiply-Imported Files

When a file with side effects is imported under multiple names:

```rust
// In crates/cribo/src/code_generator.rs

impl Bundler {
    fn should_wrap_module(&self, module_name: &str, ast: &ModModule) -> bool {
        // Always wrap if:
        // 1. Module has multiple import names (to ensure side effects run once)
        // 2. Module might be imported dynamically
        // 3. Module needs to support module attributes (__name__, __file__, etc.)

        let has_multiple_names = self.get_import_names_for_file(module_name).len() > 1;
        let has_side_effects = self.module_has_side_effects(ast);
        let might_be_dynamic = self.might_be_imported_dynamically(module_name);

        has_multiple_names || (has_side_effects && might_be_dynamic)
    }
}
```

## Implementation Plan

### Phase 1: Import Discovery (Week 1)

- [x] Add static importlib detection to ImportDiscoveryVisitor
- [x] Update DiscoveredImport to include import type
- [x] Add tests for importlib discovery

### Phase 2: Path Canonicalization (Week 1)

- [ ] Update Resolver to always canonicalize paths
- [ ] Add path canonicalization utilities
- [ ] Update existing tests for canonical paths

### Phase 3: Dependency Graph (Week 2)

- [ ] Add file-to-import-names tracking
- [ ] Update module registration for multiple names
- [ ] Add methods to query modules by file

### Phase 4: Code Generation (Week 3)

- [ ] Implement file-based deduplication in bundler
- [ ] Update import transformers for all import types
- [ ] Handle edge cases (invalid identifiers, side effects)

### Phase 5: Testing & validation (Week 4)

- [ ] Create comprehensive test fixtures
- [ ] Test with real-world packages
- [ ] Performance benchmarking

## Testing Strategy

### Test Fixtures

1. **file_deduplication_basic/**
   ```python
   # app/utils.py
   def get_name():
       return "app.utils"


   # main.py
   import utils  # May resolve to app/utils.py
   from app import utils as app_utils  # Definitely app/utils.py
   import importlib

   mod = importlib.import_module("app.utils")  # Also app/utils.py

   # All three should reference the same inlined content
   ```

2. **file_deduplication_symlinks/**
   ```
   shared/common.py -> ../lib/helpers.py

   # main.py
   from lib import helpers
   from shared import common
   # Should only include helpers.py content once
   ```

3. **importlib_edge_cases/**
   ```python
   # my-module.py (invalid identifier)
   value = 42

   # main.py
   import importlib

   mod = importlib.import_module("my-module")
   print(mod.value)  # Should work
   ```

### Expected Behavior

- Each unique file appears exactly once in the bundle
- All import forms (regular and importlib) are rewritten consistently
- No runtime module system needed - everything is statically resolved
- Circular dependencies work correctly even with multiple import names

## Performance Impact

### Benefits

- **Smaller bundles**: Each file included only once
- **Faster execution**: No runtime import resolution
- **Better tree-shaking**: Clearer dependency relationships

### Costs

- **Build time**: Path canonicalization adds syscalls
- **Memory**: Tracking multiple import names per file

### Mitigation

- Cache canonical paths during build
- Use efficient data structures (FxHashMap)
- Lazy canonicalization where possible

## Conclusion

This proposal ensures comprehensive file-based deduplication in Cribo by:

1. Treating all static imports (including importlib) uniformly
2. Tracking files by canonical path to handle symlinks and multiple import names
3. Processing each file exactly once in the bundle
4. Rewriting all import forms to use the deduplicated content
5. Maintaining correct behavior for edge cases

The solution integrates cleanly with Cribo's existing static bundling approach without requiring any runtime module system.
