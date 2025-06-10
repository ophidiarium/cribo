# Static Bundling System Design

## Overview

The static bundling feature in Cribo eliminates the use of runtime `exec()` calls by transforming Python modules into wrapper classes. This approach addresses compatibility issues with certain Python environments where `exec()` is restricted or problematic, particularly when dealing with hoisted stdlib imports.

## Problem Statement

The traditional bundling approach uses `exec()` to execute module code in isolated namespaces. This causes issues when:

- Hoisted standard library imports are not accessible within the exec'd code
- Security-restricted environments block or limit `exec()` usage
- Debugging becomes difficult due to dynamic code execution
- Performance overhead from runtime code compilation

## Solution Architecture

### Core Concept

Transform each Python module into a wrapper class where:

- Functions become static methods
- Module-level variables are stored in a `__cribo_vars` dictionary
- Complex initialization code goes into a `__cribo_init` classmethod
- Classes remain as nested classes within the wrapper

### Module Transformation

Each module `foo.bar` is transformed into a class `__cribo_module_foo_bar`:

```python
# Original module: foo/bar.py
VERSION = "1.0.0"
DEBUG = True

def helper(x):
    return x * 2

class Config:
    timeout = 30

# Complex initialization
if DEBUG:
    print("Debug mode enabled")
```

```python
# Transformed module
class __cribo_module_foo_bar:
    __cribo_vars = {
        'VERSION': "1.0.0",
        'DEBUG': True
    }
    
    @staticmethod
    def helper(x):
        return x * 2
    
    class Config:
        timeout = 30
    
    @classmethod
    def __cribo_init(cls):
        if DEBUG:
            print("Debug mode enabled")
```

### Module Facade System

After transformation, module facades are created to maintain Python's import semantics:

```python
import types

# Create module objects
foo = types.ModuleType('foo')
foo.bar = types.ModuleType('foo.bar')

# Copy attributes from wrapper to module
for __attr in dir(__cribo_module_foo_bar):
    if not __attr.startswith('_'):
        setattr(foo.bar, __attr, getattr(__cribo_module_foo_bar, __attr))

# Copy module variables
if hasattr(__cribo_module_foo_bar, '__cribo_vars'):
    for __k, __v in __cribo_module_foo_bar.__cribo_vars.items():
        setattr(foo.bar, __k, __v)

# Run initialization code
if hasattr(__cribo_module_foo_bar, '__cribo_init'):
    __cribo_module_foo_bar.__cribo_init()
```

### Import Rewriting

Import statements for bundled modules are handled specially:

1. **Simple imports** (`import foo`) - Removed entirely as module objects are pre-created
2. **From imports** (`from foo import bar`) - Converted to attribute access:
   ```python
   # Original
   from foo import bar

   # Transformed
   bar = getattr(foo, 'bar')
   ```

### Entry Module Handling

The entry module receives special treatment:

- Its code executes directly in the global scope (not wrapped in a class)
- This ensures the main script behaves as expected
- Import statements in the entry module are still rewritten to use bundled modules

## Implementation Details

### AST Transformation Rules

1. **Functions** → Static methods with `@staticmethod` decorator
2. **Classes** → Nested classes (unchanged)
3. **Simple assignments** → Stored in `__cribo_vars` if they don't reference other variables
4. **Complex assignments** → Placed in `__cribo_init` method
5. **Import statements** → Filtered out during transformation
6. **Control flow statements** → Placed in `__cribo_init` method
7. **Expression statements** → Placed in `__cribo_init` method

### Variable Reference Detection

The bundler analyzes assignment values to determine placement:

- Literals and constants → `__cribo_vars`
- Expressions with variable references → `__cribo_init`

This ensures variables are initialized in the correct order.

### Module Initialization Order

1. Transform all non-entry modules into wrapper classes
2. Create module facade objects (types.ModuleType)
3. Copy attributes from wrappers to module objects
4. Execute `__cribo_init` methods in dependency order
5. Execute entry module code directly

## Benefits

1. **No exec() calls** - Compatible with restricted environments
2. **Better debugging** - Stack traces show actual class/method names
3. **Static analysis friendly** - Tools can analyze the bundled code
4. **Performance** - No runtime code compilation overhead
5. **Security** - Reduced attack surface without dynamic execution

## Limitations

1. **Dynamic module manipulation** - Code that modifies `__dict__` directly may not work
2. **Module reload** - `importlib.reload()` won't work as expected
3. **Circular imports** - Handled but may require careful initialization ordering
4. **Bundle size** - Slightly larger due to wrapper infrastructure

## Configuration

Enable static bundling via:

```toml
# cribo.toml
[bundler]
static-bundling = true
```

Or via CLI:

```bash
cribo --entry main.py --output bundle.py --static-bundling
```

Or via environment variable:

```bash
CRIBO_STATIC_BUNDLING=true cribo --entry main.py --output bundle.py
```

## Alternative Approaches Comparison

### Approach 1: Wrapper Classes (Current Implementation)

**How it works:**

- Each module becomes a class (`__cribo_module_foo_bar`)
- Functions become static methods
- Module vars stored in `__cribo_vars` dict
- Complex initialization in `__cribo_init` method
- Module facades created with `types.ModuleType`

**Pros:**

- Preserves module namespaces exactly
- Clean separation of module initialization
- Supports all Python module features
- Easy debugging with clear module boundaries

**Cons:**

- Complex implementation
- Forward reference issues with classes
- Larger bundle size due to wrapper infrastructure
- Performance overhead from attribute copying

**Example:**

```python
# Original
class User:
    def __init__(self, name):
        self.name = name

def create_user(name):
    return User(name)

# Bundled
class __cribo_module_models:
    class User:
        def __init__(self, name):
            self.name = name
    
    @staticmethod
    def create_user(name):
        return User(name)  # NameError: User not defined
```

### Approach 2: Simple Symbol Renaming

**How it works:**

- All symbols get unique prefixed names
- Direct renaming: `foo.bar.func` → `__foo_bar__func`
- No wrapper classes or module objects
- All code in flat global namespace

**Pros:**

- Simple implementation
- No forward reference issues
- Smaller bundle size
- Better performance (no wrapper overhead)
- Similar to JavaScript bundlers (Rolldown/webpack)

**Cons:**

- Module namespaces are flattened
- Dynamic imports become difficult
- `__all__` exports need special handling
- Less Python-idiomatic

**Example:**

```python
# Original
# models.py
class User:
    def __init__(self, name):
        self.name = name

def create_user(name):
    return User(name)

# utils.py
class User:  # Name collision!
    role = "admin"

# main.py
from models import User as ModelUser
from utils import User as UtilUser

# Bundled (with renaming)
class __models__User:
    def __init__(self, name):
        self.name = name

def __models__create_user(name):
    return __models__User(name)

class __utils__User:
    role = "admin"

# main.py code
ModelUser = __models__User
UtilUser = __utils__User
```

### Approach 3: Hybrid Module Wrappers (Inspired by Rolldown)

**How it works:**

- Modules wrapped in initialization functions
- Lazy initialization to handle circular dependencies
- Module namespace objects for clean boundaries
- Symbol renaming only for conflicts

**Pros:**

- Best of both approaches
- Handles circular dependencies well
- Supports tree shaking
- Maintains module boundaries
- Uses Python's native module system

**Cons:**

- More complex than simple renaming
- Requires careful initialization ordering
- Still has some wrapper overhead

**Example:**

```python
# Bundled output
import sys
import types
import hashlib

# Module registry maps original names to synthetic names
__cribo_modules = {
    'models': '__cribo_7a3f9b_models',        # Hash of 'src/models.py'
    'utils': '__cribo_4e8c12_utils',          # Hash of 'lib/utils.py'
    'models.user': '__cribo_9d2f5a_models_user',  # Hash of 'src/models/user.py'
}

def __cribo_get_module_name(original_name):
    """Get the synthetic module name for bundled modules."""
    return __cribo_modules.get(original_name, original_name)

def __cribo_init_7a3f9b_models():
    # Use synthetic name to avoid conflicts
    synthetic_name = '__cribo_7a3f9b_models'
    
    if synthetic_name in sys.modules:
        return sys.modules[synthetic_name]
    
    # Create module with synthetic name
    module = types.ModuleType(synthetic_name)
    module.__file__ = 'src/models.py'  # Original path for debugging
    module.__package__ = ''
    module.__name__ = synthetic_name
    
    # Register in sys.modules immediately
    sys.modules[synthetic_name] = module
    
    # Also register under original name for internal imports
    sys.modules['models'] = module
    
    # Define module contents
    class User:
        def __init__(self, name):
            self.name = name
    
    def create_user(name):
        return User(name)
    
    # Set module attributes
    module.User = User
    module.create_user = create_user
    module.__all__ = ['User', 'create_user']
    
    return module

def __cribo_init_4e8c12_utils():
    synthetic_name = '__cribo_4e8c12_utils'
    
    if synthetic_name in sys.modules:
        return sys.modules[synthetic_name]
    
    module = types.ModuleType(synthetic_name)
    module.__file__ = 'lib/utils.py'
    sys.modules[synthetic_name] = module
    sys.modules['utils'] = module
    
    class User:
        role = "admin"
    
    module.User = User
    module.__all__ = ['User']
    
    return module

# Install custom import hook for bundled modules
class CriboBundledFinder:
    """Import hook that redirects bundled module imports to synthetic names."""
    
    def find_spec(self, fullname, path, target=None):
        if fullname in __cribo_modules:
            # This is a bundled module
            synthetic_name = __cribo_modules[fullname]
            if synthetic_name not in sys.modules:
                # Initialize the module
                init_func = globals().get(f'__cribo_init_{synthetic_name.split("_")[2]}_{fullname.replace(".", "_")}')
                if init_func:
                    init_func()
            
            # Return the spec for the synthetic module
            import importlib.util
            return importlib.util.find_spec(synthetic_name)
        return None

# Install the import hook
sys.meta_path.insert(0, CriboBundledFinder())

# Initialize modules
__cribo_init_7a3f9b_models()
__cribo_init_4e8c12_utils()

# Now imports work with proper isolation
import models  # Gets __cribo_7a3f9b_models
import utils   # Gets __cribo_4e8c12_utils

# Third-party 'models' package would still work
import models as third_party_models  # Would bypass our finder if not in __cribo_modules
```

**Hash Generation Strategy:**

```rust
// In the bundler
fn generate_module_hash(file_path: &Path, entry_point: &Path) -> String {
    // Get relative path from entry point
    let relative_path = file_path
        .strip_prefix(entry_point.parent().unwrap())
        .unwrap_or(file_path);

    // Create stable hash from relative path
    let mut hasher = Sha256::new();
    hasher.update(relative_path.to_string_lossy().as_bytes());
    let hash = hasher.finalize();

    // Take first 6 chars of hex for readability
    format!("{:x}", hash)[..6].to_string()
}

// Generate synthetic module name
fn get_synthetic_module_name(module_name: &str, file_path: &Path, entry_point: &Path) -> String {
    let hash = generate_module_hash(file_path, entry_point);
    format!("__cribo_{}_{}", hash, module_name.replace('.', "_"))
}
```

**Advanced Example with Circular Dependencies:**

```python
# Module registry with hashed names
__cribo_modules = {
    'models': '__cribo_7a3f9b_models',
    'models.user': '__cribo_9d2f5a_models_user',
    'models.post': '__cribo_3b8e7c_models_post',
}

# Handling circular imports between models.user and models.post
def __cribo_init_9d2f5a_models_user():
    synthetic_name = '__cribo_9d2f5a_models_user'
    
    if synthetic_name in sys.modules:
        return sys.modules[synthetic_name]
    
    # Create and register immediately
    module = types.ModuleType(synthetic_name)
    module.__file__ = 'src/models/user.py'
    module.__name__ = synthetic_name
    sys.modules[synthetic_name] = module
    sys.modules['models.user'] = module  # Also register original name
    
    # Can safely import post now - it will get partial module if circular
    from models.post import Post  # Works even if post imports user!
    
    class User:
        def __init__(self, name):
            self.name = name
            self.posts = []
        
        def add_post(self, title):
            post = Post(title, author=self)
            self.posts.append(post)
            return post
    
    module.User = User
    return module

def __cribo_init_3b8e7c_models_post():
    synthetic_name = '__cribo_3b8e7c_models_post'
    
    if synthetic_name in sys.modules:
        return sys.modules[synthetic_name]
    
    module = types.ModuleType(synthetic_name)
    module.__file__ = 'src/models/post.py'
    module.__name__ = synthetic_name
    sys.modules[synthetic_name] = module
    sys.modules['models.post'] = module
    
    # Import User - gets partial or complete module
    from models.user import User
    
    class Post:
        def __init__(self, title, author):
            self.title = title
            self.author = author  # This is a User instance
    
    module.Post = Post
    return module

# Benefits of hash-based approach:
# 1. No conflicts with third-party 'models' packages
# 2. Deterministic names based on file paths
# 3. Can bundle multiple projects without name collisions
# 4. Import hooks ensure correct module resolution
```

**Simplified Import Hook Implementation:**

```python
class CriboBundledFinder:
    """Minimal import hook for bundled modules."""
    
    def __init__(self, module_registry, init_functions):
        self.module_registry = module_registry
        self.init_functions = init_functions
    
    def find_spec(self, fullname, path, target=None):
        if fullname in self.module_registry:
            synthetic_name = self.module_registry[fullname]
            
            # Initialize if needed
            if synthetic_name not in sys.modules:
                init_func = self.init_functions.get(synthetic_name)
                if init_func:
                    init_func()
            
            # Return existing spec
            import importlib.util
            return importlib.util.find_spec(synthetic_name)
        return None

# Register all init functions
__cribo_init_functions = {
    '__cribo_7a3f9b_models': __cribo_init_7a3f9b_models,
    '__cribo_4e8c12_utils': __cribo_init_4e8c12_utils,
    '__cribo_9d2f5a_models_user': __cribo_init_9d2f5a_models_user,
    '__cribo_3b8e7c_models_post': __cribo_init_3b8e7c_models_post,
}

# Install hook
sys.meta_path.insert(0, CriboBundledFinder(__cribo_modules, __cribo_init_functions))
```

### Recommendation

For Cribo's next iteration, consider the **Hybrid Module Wrappers** approach because:

1. It solves the forward reference problem elegantly
2. Maintains Python's module semantics
3. Handles circular dependencies naturally
4. Enables future optimizations (tree shaking, lazy loading)
5. Aligns with proven JavaScript bundler architectures

The simple renaming approach is tempting for its simplicity, but Python's dynamic nature and module system expectations make the hybrid approach more suitable for maintaining compatibility while achieving the goal of static bundling.

## Source Map Support

### Overview

Source maps enable debugging of bundled code by mapping locations in the bundle back to original source files. Cribo should adopt the JavaScript Source Map v3 specification for Python bundles.

### Source Map v3 Format

```json
{
  "version": 3,
  "file": "bundle.py",
  "sourceRoot": "",
  "sources": ["src/main.py", "src/utils.py", "src/models/user.py"],
  "names": ["helper", "User", "create_user"],
  "mappings": "AAAA,SAASA,QAAQ;AACjB...",
  "sourcesContent": ["# Original source of main.py", "# Original source of utils.py", ...]
}
```

### Implementation Strategy

#### 1. Track Transformations During Bundling

```rust
#[derive(Debug, Clone)]
struct SourceLocation {
    file_path: String,
    line: usize,
    column: usize,
}

#[derive(Debug)]
struct MappingSegment {
    generated_line: usize,
    generated_column: usize,
    source_file_index: usize,
    source_line: usize,
    source_column: usize,
    name_index: Option<usize>,
}

struct SourceMapBuilder {
    mappings: Vec<MappingSegment>,
    sources: Vec<String>,
    names: Vec<String>,
    sources_content: Vec<String>,
}
```

#### 2. During AST Transformation

When transforming AST nodes, preserve original location information:

```rust
// In the bundler
fn transform_function(&mut self, func: &mut StmtFunctionDef, original_loc: SourceLocation) {
    // Transform the function
    let new_name = self.rename_symbol(&func.name);

    // Record the mapping
    self.source_map.add_mapping(MappingSegment {
        generated_line: func.range.start.line,
        generated_column: func.range.start.column,
        source_file_index: self.get_source_index(&original_loc.file_path),
        source_line: original_loc.line,
        source_column: original_loc.column,
        name_index: Some(self.get_name_index(&func.name.to_string())),
    });
}
```

#### 3. VLQ Encoding for Mappings

The mappings string uses Base64 VLQ (Variable Length Quantity) encoding:

```rust
fn encode_vlq(value: i32) -> String {
    const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    let mut value = if value < 0 {
        ((-value) << 1) | 1
    } else {
        value << 1
    };

    loop {
        let mut digit = value & 0x1F;
        value >>= 5;
        if value > 0 {
            digit |= 0x20; // Set continuation bit
        }
        encoded.push(BASE64_CHARS[digit as usize] as char);
        if value == 0 {
            break;
        }
    }
    encoded
}
```

#### 4. Python Exception Integration

Add a custom exception handler that reads source maps:

```python
# At the end of bundled file
import sys
import json
import traceback

_SOURCE_MAP = json.loads('''
{
  "version": 3,
  "file": "bundle.py",
  "sources": ["main.py", "utils.py"],
  "mappings": "...",
  "sourcesContent": [...]
}
''')

def _cribo_exception_handler(exc_type, exc_value, exc_traceback):
    """Map bundled code locations back to original sources."""
    tb_lines = []
    
    for frame in traceback.extract_tb(exc_traceback):
        if frame.filename == __file__:  # This is bundled code
            # Decode source map to find original location
            orig_file, orig_line = _decode_source_location(frame.lineno, frame.col_offset)
            if orig_file:
                # Show original source context
                source_line = _get_source_line(orig_file, orig_line)
                tb_lines.append(f'  File "{orig_file}", line {orig_line}')
                if source_line:
                    tb_lines.append(f'    {source_line.strip()}')
            else:
                # Fallback to bundled location
                tb_lines.append(f'  File "{frame.filename}", line {frame.lineno}')
                tb_lines.append(f'    {frame.line}')
        else:
            # External file, show as-is
            tb_lines.append(f'  File "{frame.filename}", line {frame.lineno}')
            tb_lines.append(f'    {frame.line}')
    
    print(f"Traceback (most recent call last):")
    print('\n'.join(tb_lines))
    print(f"{exc_type.__name__}: {exc_value}")

sys.excepthook = _cribo_exception_handler
```

#### 5. External Source Map Files

For production, source maps should be in separate files:

```python
# bundle.py
#!/usr/bin/env python3
# sourceMappingURL=bundle.py.map

# ... bundled code ...
```

The source map loader would check for:

1. Inline source map comment
2. External `.map` file
3. HTTP header `X-SourceMap` (for web-served bundles)

### Benefits of Source Maps

1. **Accurate debugging** - IDEs can set breakpoints in original files
2. **Better error messages** - Stack traces show original locations
3. **Profiling support** - Performance tools can attribute time to original code
4. **Code coverage** - Map coverage back to source files
5. **Integration with existing tools** - Many tools already support Source Map v3

### Source Maps with Hybrid Module Approach

The hybrid module wrapper approach is particularly well-suited for source mapping because:

1. **Module boundaries are preserved** - Each init function corresponds to one source file
2. **Line numbers are mostly preserved** - Module content is copied almost verbatim
3. **Synthetic names are traceable** - Hash-based names map back to file paths
4. **Clean transformation** - Predictable wrapping pattern makes mapping easier

**Example with Source Mapping:**

```python
# bundled.py with source map annotations
import sys
import types

# sourceMappingURL=bundled.py.map

__cribo_modules = {
    'models': '__cribo_7a3f9b_models',  # src/models.py
    'utils': '__cribo_4e8c12_utils',    # lib/utils.py
}

def __cribo_init_7a3f9b_models():
    synthetic_name = '__cribo_7a3f9b_models'
    if synthetic_name in sys.modules:
        return sys.modules[synthetic_name]
    
    module = types.ModuleType(synthetic_name)
    module.__file__ = 'src/models.py'
    sys.modules[synthetic_name] = module
    sys.modules['models'] = module
    
    # === START src/models.py line 1 ===
    class User:
        def __init__(self, name):  # line 2
            self.name = name        # line 3
    
    def create_user(name):          # line 5
        return User(name)           # line 6
    # === END src/models.py ===
    
    module.User = User
    module.create_user = create_user
    return module
```

**Source Map Generation Strategy:**

```rust
impl SourceMapGenerator {
    fn generate_for_hybrid_module(&mut self, module: &Module, wrapper_fn_start_line: usize) {
        // Track the wrapper function overhead
        let wrapper_overhead = 9; // Lines before module content starts

        // Map each line in the original module
        for (original_line, generated_line) in module.lines.iter().enumerate() {
            self.add_mapping(Mapping {
                generated_line: wrapper_fn_start_line + wrapper_overhead + original_line,
                generated_column: 4, // Indentation inside function
                source_file: module.file_path,
                source_line: original_line + 1,
                source_column: 0,
            });
        }

        // Map the module attribute assignments
        let attr_start = wrapper_fn_start_line + wrapper_overhead + module.lines.len();
        for (i, attr) in module.exports.iter().enumerate() {
            self.add_name_mapping(
                attr_start + i,
                module.file_path,
                attr.original_line,
                attr.name,
            );
        }
    }
}
```

**Enhanced Exception Handler with Source Maps:**

```python
def _cribo_exception_handler(exc_type, exc_value, exc_traceback):
    """Enhanced handler that uses module.__file__ attributes."""
    tb_lines = []
    
    for frame in traceback.extract_tb(exc_traceback):
        # Check if this is a cribo synthetic module
        module_name = frame.name  # e.g., '__cribo_7a3f9b_models'
        
        if module_name.startswith('__cribo_') and module_name in sys.modules:
            # Get the original file from module.__file__
            original_file = sys.modules[module_name].__file__
            
            # Calculate original line number
            # The source map tells us the offset
            original_line = _map_line_number(module_name, frame.lineno)
            
            tb_lines.append(f'  File "{original_file}", line {original_line}')
            
            # Get original source line if available
            if original_file in _SOURCE_MAP['sourcesContent']:
                source_lines = _SOURCE_MAP['sourcesContent'][original_file]
                if 0 < original_line <= len(source_lines):
                    tb_lines.append(f'    {source_lines[original_line-1].strip()}')
        else:
            # Regular module
            tb_lines.append(f'  File "{frame.filename}", line {frame.lineno}')
            if frame.line:
                tb_lines.append(f'    {frame.line}')
    
    print("Traceback (most recent call last):")
    print('\n'.join(tb_lines))
    print(f"{exc_type.__name__}: {exc_value}")

sys.excepthook = _cribo_exception_handler
```

**Advantages for Source Mapping:**

1. **Clear boundaries** - Each module's code is contained within its init function
2. **Preserved structure** - Classes, functions, and statements maintain relative positions
3. **Traceable names** - Synthetic module names contain hash that maps to file path
4. **Module metadata** - `module.__file__` provides original path without decoding
5. **Minimal transformation** - Most lines map 1:1 with just indentation changes

**IDE Integration Example:**

```python
# VSCode launch.json configuration
{
    "type": "python",
    "request": "launch",
    "name": "Debug Cribo Bundle",
    "program": "${workspaceFolder}/bundle.py",
    "sourceMaps": true,
    "sourceMapPathOverrides": {
        "__cribo_*": "${workspaceFolder}/${relativeFile}"
    }
}
```

## PyBake Comparison

### PyBake's Module Loading Approach

PyBake takes a different approach to bundling Python modules, using a compressed virtual filesystem and runtime import hooks.

**Key Components:**

1. **DictFileSystem**: A virtual filesystem that stores modules in a nested dictionary structure
2. **AbstractImporter**: PEP 302 compliant import hook that intercepts module imports
3. **Compressed Storage**: Modules are stored as base64-encoded, zlib-compressed JSON blob
4. **Runtime Module Creation**: Uses `types.ModuleType` and `exec()` to create modules dynamically

**How PyBake Works:**

1. **Bundle Creation**:
   ```python
   # PyBake stores modules in a compressed blob
   blob = (execable_code, preload_modules, dict_filesystem_tree)
   json_blob = json.dumps(blob, sort_keys=True)
   zlib_blob = zlib.compress(json_blob.encode('utf-8'))
   b64_blob = binascii.b2a_base64(zlib_blob)
   ```

2. **Runtime Loading**:
   ```python
   # Modules are created dynamically
   mod = sys.modules.setdefault(fullname, types.ModuleType(fullname))
   mod.__file__ = full_path
   mod.__loader__ = self

   # Code is executed in module namespace
   exec(compile(source, full_path, 'exec'), mod.__dict__)
   ```

3. **Import Interception**:
   ```python
   # Custom finder installed in sys.meta_path
   class AbstractImporter:
       def find_module(self, fullname, path=None):
           if self._full_path(fullname):
               return self
       
       def load_module(self, fullname):
           # Load from virtual filesystem
           source = self._read_file(full_path)
           exec(compile(source, full_path, 'exec'), mod.__dict__)
   ```

**Advantages of PyBake's Approach:**

1. **Compact Storage**: Single compressed blob contains all modules
2. **Standard Import Semantics**: Uses Python's import system directly
3. **Dynamic Loading**: Modules loaded on-demand when imported
4. **No AST Transformation**: Original code executed as-is
5. **Clean Module Isolation**: Each module gets proper `__dict__` namespace

**Disadvantages:**

1. **Uses exec()**: The very problem we're trying to avoid in Cribo
2. **Runtime Overhead**: Decompression and module creation at runtime
3. **No Static Analysis**: Bundled code is compressed, not analyzable
4. **No Tree Shaking**: All modules included in blob
5. **Security Concerns**: Dynamic code execution

### Cribo's Hybrid Static Bundler vs PyBake

| Feature                | Cribo Hybrid Bundler                  | PyBake                           |
| ---------------------- | ------------------------------------- | -------------------------------- |
| **Storage**            | Uncompressed Python code              | Compressed JSON blob             |
| **Module Creation**    | `sys.modules` with synthetic names    | `types.ModuleType` with exec()   |
| **Import Handling**    | PEP 302 hooks + static transformation | PEP 302 hooks only               |
| **Code Execution**     | Module init functions (no exec)       | Direct exec() in module.**dict** |
| **AST Transformation** | Yes, for imports and module wrapping  | No transformation                |
| **Debugging**          | Source maps + clear module boundaries | Limited, compressed code         |
| **Performance**        | Faster startup (no decompression)     | Slower (decompression + exec)    |
| **Security**           | No exec() calls                       | Uses exec() extensively          |
| **Tree Shaking**       | Supported                             | Not supported                    |
| **Module Conflicts**   | Hash-based naming prevents conflicts  | Relies on import path resolution |

### Key Insights from PyBake

1. **Virtual Filesystem Benefits**: PyBake's DictFileSystem provides clean module organization
2. **PEP 302 Import Hooks**: Both Cribo and PyBake use this standard Python mechanism, but in different ways:
   - **PyBake**: Uses hooks to load from compressed blob at runtime
   - **Cribo**: Uses hooks to redirect imports to pre-initialized synthetic modules
3. **Module Preloading**: PyBake preloads core modules for better performance
4. **Compression Trade-offs**: Smaller file size vs runtime overhead

### Similarities and Differences in PEP 302 Usage

Both Cribo and PyBake implement PEP 302 import hooks, but with fundamentally different approaches:

**Cribo's Implementation:**

```python
class CriboBundledFinder:
    def find_spec(self, fullname, path, target=None):
        if fullname in __cribo_modules:
            # Redirect to pre-initialized synthetic module
            synthetic_name = __cribo_modules[fullname]
            if synthetic_name not in sys.modules:
                # Call init function (no exec!)
                init_func = __cribo_init_functions[synthetic_name]
                init_func()
            return importlib.util.find_spec(synthetic_name)
```

**PyBake's Implementation:**

```python
class AbstractImporter:
    def load_module(self, fullname):
        # Create module and execute code dynamically
        mod = types.ModuleType(fullname)
        source = self._read_file(full_path)  # From compressed blob
        exec(compile(source, full_path, 'exec'), mod.__dict__)
        return mod
```

The key difference is that Cribo's hooks redirect to **pre-transformed, statically bundled code**, while PyBake's hooks **dynamically execute compressed source code**.

### Incorporating PyBake Ideas into Cribo

While PyBake's exec-based approach doesn't align with Cribo's goals, several concepts could enhance our hybrid bundler:

1. **Optional Compression**: Add opt-in compression for deployment scenarios (but decompress at build time, not runtime)
2. **Virtual Filesystem Metadata**: Store module metadata similar to DictFileSystem for better introspection
3. **Import Hook Optimization**: PyBake's simpler hook structure could inspire performance improvements
4. **Preload Critical Modules**: Initialize frequently-used modules upfront like PyBake does

### Why Cribo's Approach is Superior for Static Bundling

1. **No exec() Required**: Compatible with restricted environments
2. **Static Analysis Friendly**: Bundled code remains analyzable
3. **Better Performance**: No runtime decompression or compilation
4. **Source Map Support**: Full debugging capabilities
5. **Tree Shaking**: Only include actually used code
6. **Type Safety**: Preserves type hints and static typing

The hybrid static bundler in Cribo represents an evolution beyond PyBake's approach, providing the benefits of bundling without sacrificing security, performance, or debuggability.

### Implementation Phases

**Phase 1: Basic Mappings**

- Track line numbers only
- Simple 1:1 mapping for unchanged lines
- Handle basic transformations (renames, imports)

**Phase 2: Column Mappings**

- Add column-level precision
- Track expression-level transformations
- Support inline source maps

**Phase 3: Advanced Features**

- Name mappings for renamed symbols
- Scope tracking for better variable resolution
- Support for source map composition (bundling bundles)

**Phase 4: Tooling Integration**

- VSCode extension for debugging bundled Python
- pytest plugin for mapped test failures
- Coverage.py integration for mapped coverage reports

### Example Source Map Generation

```rust
impl SourceMapBuilder {
    fn generate(&self) -> SourceMap {
        let mut mappings = String::new();
        let mut prev_generated_line = 0;
        let mut prev_generated_column = 0;
        let mut prev_source_index = 0;
        let mut prev_source_line = 0;
        let mut prev_source_column = 0;

        for segment in &self.mappings {
            // Encode relative offsets
            mappings.push_str(&encode_vlq(
                segment.generated_column as i32 - prev_generated_column as i32,
            ));
            mappings.push_str(&encode_vlq(
                segment.source_file_index as i32 - prev_source_index as i32,
            ));
            mappings.push_str(&encode_vlq(
                segment.source_line as i32 - prev_source_line as i32,
            ));
            mappings.push_str(&encode_vlq(
                segment.source_column as i32 - prev_source_column as i32,
            ));

            // Update previous values
            prev_generated_column = segment.generated_column;
            prev_source_index = segment.source_file_index;
            prev_source_line = segment.source_line;
            prev_source_column = segment.source_column;

            // Add separator
            if segment.generated_line != prev_generated_line {
                mappings.push(';');
                prev_generated_line = segment.generated_line;
                prev_generated_column = 0;
            } else {
                mappings.push(',');
            }
        }

        SourceMap {
            version: 3,
            file: "bundle.py".to_string(),
            sources: self.sources.clone(),
            names: self.names.clone(),
            mappings,
            sources_content: Some(self.sources_content.clone()),
        }
    }
}
```

### Testing Source Maps

```python
# test_source_maps.py
def test_exception_mapping():
    # Create a bundle with known transformations
    bundle = create_test_bundle()
    
    # Trigger an exception at a known location
    with pytest.raises(ValueError) as exc_info:
        bundle.execute()
    
    # Verify the mapped traceback points to original file
    tb = exc_info.traceback
    original_location = map_to_source(tb[-1])
    
    assert original_location.file == "src/utils.py"
    assert original_location.line == 42
    assert original_location.column == 15
```

## Future Enhancements

1. **Optimization passes** - Remove unnecessary wrapper overhead for simple modules
2. **Source maps** - Full implementation as described above
3. **Lazy module loading** - Defer module initialization until first access
4. **Tree shaking** - Remove unused module attributes from bundles
5. **Type preservation** - Maintain type hints in transformed code
6. **Import function transformation** - Support for `__import__()` and `importlib.import_module()`
7. **Canonical name system** - Implement Rolldown-style symbol mapping
8. **Module splitting** - Support for dynamic imports with code splitting
9. **Debug mode** - Generate bundles with debugging helpers and assertions
10. **Source map composition** - Support bundling pre-bundled code with composed maps

## Testing Strategy

1. **Unit tests** - Test individual transformation rules
2. **Integration tests** - Test full bundling pipeline
3. **Compatibility tests** - Ensure feature parity with exec-based bundling
4. **Performance benchmarks** - Compare with traditional bundling
5. **Real-world projects** - Test with complex Python applications
