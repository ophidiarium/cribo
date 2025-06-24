# Resolving to Absolute Paths for Deduplication

## Problem Statement

The current implementation of Cribo's module resolution system has several issues that could lead to incorrect bundling:

1. **Module deduplication is based on module names only** - But Python deduplicates by the full import name in `sys.modules`, meaning the same file can be loaded multiple times under different names
2. **Resolved file paths are not canonicalized** - This could cause issues with symlinks or relative paths
3. **The resolver lacks context about where imports are found** - The context-aware methods exist but aren't used during discovery

## Current Implementation Analysis

### 1. Module Deduplication by Name Only

In `crates/cribo/src/cribo_graph.rs:719-723`, modules are deduplicated by name:

```rust
pub fn add_module(&mut self, name: String, path: PathBuf) -> ModuleId {
    // Check if module already exists
    if let Some(&id) = self.module_names.get(&name) {
        return id;
    }
    // ... continues to add new module
```

The `module_paths` HashMap exists but is not consulted for deduplication:

- `module_paths: FxHashMap<PathBuf, ModuleId>` (line 671)

### 2. Inconsistent Path Canonicalization

#### Search Directories ARE Canonicalized

In `crates/cribo/src/resolver.rs:298-302`:

```rust
if let Some(entry_dir) = &self.entry_dir {
    if let Ok(canonical) = entry_dir.canonicalize() {
        unique_dirs.insert(canonical);
    } else {
        unique_dirs.insert(entry_dir.clone());
    }
}
```

#### Resolved Paths ARE NOT Canonicalized

In `crates/cribo/src/resolver.rs:498-501`:

```rust
let package_init = current_path.join(part).join("__init__.py");
if package_init.is_file() {
    debug!("Found package at: {package_init:?}");
    return Ok(Some(package_init));  // Not canonicalized!
}
```

### 3. Discovery Phase Deduplication

In `crates/cribo/src/orchestrator.rs:607-610`:

```rust
if processed_modules.contains(&module_name) {
    debug!("Module {module_name} already discovered, skipping");
    continue;
}
```

Again, deduplication is by module name only.

### 4. Missing Import Context During Resolution

The resolver has a context-aware method but it's rarely used:

In `crates/cribo/src/resolver.rs:365-369`:

```rust
pub fn resolve_module_path_with_context(
    &mut self,
    module_name: &str,
    current_module_path: Option<&Path>,  // This would help with relative imports!
) -> Result<Option<PathBuf>>
```

But during discovery (`crates/cribo/src/orchestrator.rs:1297`):

```rust
// Only passes module name, no context about where the import was found
if let Ok(Some(import_path)) = params.resolver.resolve_module_path(import) {
```

This means:

- The resolver can't properly handle relative imports
- The orchestrator implements its own `resolve_relative_import` method
- The same import from different files might resolve differently but the resolver doesn't know

## Problematic Scenarios

### Scenario 1: Symlinks

```
project/
├── src/
│   └── utils.py
└── lib/
    └── utils.py -> ../src/utils.py  (symlink)
```

If both `src` and `lib` are in the search path, the same file could be imported as:

- `utils` (from `src/utils.py`)
- `utils` (from `lib/utils.py`)

### Scenario 2: Multiple Import Paths

```
project/
├── app/
│   ├── __init__.py
│   └── helpers.py
└── main.py
```

The same file `app/helpers.py` could be imported as:

- `helpers` (if `app/` is in search path)
- `app.helpers` (if project root is in search path)

### Scenario 3: Relative vs Absolute Paths

During development, the same file might be resolved through:

- `/Users/dev/project/src/utils.py` (absolute)
- `../project/src/utils.py` (relative from another location)

### Scenario 4: Module Name vs File Path Deduplication

Python deduplicates modules by their **import name** (the key in `sys.modules`), not by file path. This means:

```python
# If package/ is in sys.path
import submodule  # Creates sys.modules['submodule']

# Normal package import  
from package import submodule  # Creates sys.modules['package.submodule']

# These are TWO different module objects, even from the same file!
```

This was demonstrated with the `importlib_deduplication` fixture.

## Proposed Fix

### 1. Canonicalize All Resolved Paths

Modify `crates/cribo/src/resolver.rs` to canonicalize paths before returning:

```rust
// In resolve_in_directory method (around line 501)
if package_init.is_file() {
    debug!("Found package at: {package_init:?}");
    let canonical_path = package_init.canonicalize()
        .unwrap_or_else(|_| package_init);
    return Ok(Some(canonical_path));
}

// Similar changes for module files (line 508) and namespace packages (line 516)
```

### 2. Add File Path Deduplication

Modify `crates/cribo/src/cribo_graph.rs:add_module` to check both name and path:

```rust
pub fn add_module(&mut self, name: String, path: PathBuf) -> ModuleId {
    // First check if module name already exists
    if let Some(&id) = self.module_names.get(&name) {
        return id;
    }

    // Canonicalize the path for deduplication
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());

    // Check if this canonical path is already registered
    if let Some(&existing_id) = self.module_paths.get(&canonical_path) {
        // Same file but different module name - need to handle this case
        log::warn!(
            "File {:?} already registered as module {:?}, requested as {:?}",
            canonical_path,
            self.modules.get(&existing_id).map(|m| &m.module_name),
            name
        );
        // Could either return existing_id or create alias - needs design decision
        return existing_id;
    }

    // Continue with creating new module...
}
```

### 3. Handle Module Aliases

For cases where the same file can be imported under different names, we need to decide:

1. **Option A**: Treat them as the same module (return same ModuleId)
   - Pro: True deduplication, single processing
   - Con: Import rewriting becomes complex

2. **Option B**: Create module aliases
   - Pro: Preserves different import names
   - Con: Need to track alias relationships

### 4. Update Resolution Cache

Ensure the module cache also uses canonical paths:

```rust
// In resolve_module_path_with_context
if let Some(resolved_path) = self.resolve_in_directory(search_dir, &descriptor)? {
    let canonical_path = resolved_path.canonicalize()
        .unwrap_or_else(|_| resolved_path.clone());
    self.module_cache
        .insert(module_name.to_string(), Some(canonical_path.clone()));
    return Ok(Some(canonical_path));
}
```

### 5. Pass Import Context to Resolver

Update the discovery phase to use context-aware resolution:

```rust
// In orchestrator.rs, track current file being processed
let imports_with_context = self.extract_all_imports_with_context(&module_path, Some(params.resolver))?;

// When resolving imports, pass the current module's path
if let Ok(Some(import_path)) = params.resolver.resolve_module_path_with_context(
    import, 
    Some(&module_path)  // Pass context!
) {
    // Process import...
}
```

This would enable:

- Proper relative import resolution in the resolver
- Context-aware caching
- Better deduplication across different import contexts

## Implementation Steps

1. **Add path canonicalization helper**:
   ```rust
   fn canonicalize_path(path: PathBuf) -> PathBuf {
       path.canonicalize().unwrap_or_else(|e| {
           log::debug!("Failed to canonicalize {:?}: {}", path, e);
           path
       })
   }
   ```

2. **Update resolver to return canonical paths**
3. **Add file-path based deduplication to CriboGraph**
4. **Pass import context during discovery phase**
5. **Add tests for symlink scenarios**
6. **Handle edge cases** (missing files, permission errors)

## Testing Strategy

1. **Unit tests** for path canonicalization
2. **Integration tests** with symlinks
3. **Fixture tests** for multiple import paths scenario
4. **Performance tests** to ensure canonicalization doesn't slow down resolution

## Considerations

1. **Performance**: `canonicalize()` involves filesystem calls
2. **Platform differences**: Windows vs Unix path handling
3. **Error handling**: Files that exist during resolution but disappear
4. **Backwards compatibility**: Existing projects may rely on current behavior

## Alternative Approaches

1. **Content-based deduplication**: Use file content hash instead of path
2. **Lazy canonicalization**: Only canonicalize when conflicts detected
3. **Configuration option**: Allow users to enable/disable strict deduplication

## Recommendation

Based on our testing with Python's actual behavior:

1. **Path canonicalization is still important** for handling symlinks and ensuring consistent file identification
2. **Module deduplication must consider the import name**, not just the file path - the same file can legitimately be imported under different module names
3. **Context-aware resolution** should be implemented to properly handle relative imports

The key insight is that Python's module system is more complex than simple file-based deduplication. A module's identity is determined by its name in `sys.modules`, not its file path. Cribo should:

- Track both the import name and the canonical file path for each module
- Allow the same file to be bundled multiple times if imported under different names
- Use the resolver's context-aware methods during discovery to handle relative imports correctly
