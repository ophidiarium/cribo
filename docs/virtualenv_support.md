# VIRTUAL_ENV Support in Serpen

This document describes how Serpen leverages the `VIRTUAL_ENV` environment variable to improve import classification accuracy.

## Overview

Serpen supports the `VIRTUAL_ENV` environment variable to enhance third-party import detection and classification. Unlike `PYTHONPATH` which is used for first-party module discovery, `VIRTUAL_ENV` is specifically used to identify third-party packages installed in virtual environments.

## Key Architectural Differences

### PYTHONPATH vs VIRTUAL_ENV

- **PYTHONPATH**: Used for **first-party module discovery**
  - Directories in PYTHONPATH are scanned for modules that should be bundled
  - Modules found via PYTHONPATH are classified as `FirstParty`
  - These modules are included in the bundling process

- **VIRTUAL_ENV**: Used for **third-party dependency detection**
  - VIRTUAL_ENV directories are **NOT** scanned for bundling
  - Used only to improve import classification accuracy
  - Modules found in VIRTUAL_ENV are classified as `ThirdParty`
  - These modules are **excluded** from bundling but may be listed in requirements.txt

## How It Works

Serpen uses a two-tier approach for virtual environment detection:

### 1. Explicit VIRTUAL_ENV (Highest Priority)

When `VIRTUAL_ENV` is set, Serpen uses the specified path directly:

- **Windows**: `%VIRTUAL_ENV%\Lib\site-packages`
- **Unix-like**: `$VIRTUAL_ENV/lib/python*/site-packages`

### 2. Fallback Detection (When VIRTUAL_ENV is not set)

When `VIRTUAL_ENV` is not set, Serpen automatically searches the current working directory for common virtual environment directory names:

- `.venv` (most common in modern Python development)
- `venv`
- `env`
- `.virtualenv`
- `virtualenv`

Serpen validates each directory by checking for the expected site-packages structure before using it.

### 3. Import Classification

Once virtual environment packages are detected, Serpen improves import classification:

- If an import matches a package in a virtual environment → `ThirdParty`
- If an import matches a first-party module → `FirstParty`
- If an import is a standard library module → `StandardLibrary`
- Unknown imports default to → `ThirdParty`

## Usage Examples

### Automatic Detection (Activated Virtual Environment)

When running within an activated virtual environment:

```bash
# VIRTUAL_ENV is automatically set by the virtual environment
source venv/bin/activate  # Unix
# or
venv\Scripts\activate     # Windows

serpen --entry my_script.py --output bundle.py
```

### Automatic Detection (Fallback)

When VIRTUAL_ENV is not set, Serpen automatically detects common virtual environment directories:

```bash
# No activated virtual environment, but .venv directory exists
ls -la
# drwxr-xr-x  .venv/

serpen --entry my_script.py --output bundle.py  # Automatically detects .venv
```

### Manual Override

You can manually specify the virtual environment path:

```bash
VIRTUAL_ENV=/path/to/my/venv serpen --entry my_script.py --output bundle.py
```

## Virtual Environment Detection Priority

### Priority Order

1. **Explicit VIRTUAL_ENV** (highest priority) - Environment variable or manual override
2. **Fallback Detection** (lower priority) - Automatic scanning of common directory names

### Fallback Directory Search Order

When VIRTUAL_ENV is not set, Serpen searches for these directories in order:

1. `.venv` (preferred for modern Python projects)
2. `venv`
3. `env`
4. `.virtualenv`
5. `virtualenv`

### Multiple Virtual Environments

If multiple virtual environment directories exist, Serpen will scan **all** of them for packages:

```bash
# Directory structure:
# ├── .venv/          (contains requests)
# ├── venv/           (contains numpy)
# └── env/            (contains flask)

serpen --entry app.py --output bundle.py
# All packages from .venv, venv, and env are detected as third-party
```

### Validation

Serpen validates each potential virtual environment by:

1. Checking directory exists and is readable
2. Verifying expected site-packages structure:
   - **Windows**: `Lib/site-packages`
   - **Unix-like**: `lib/python*/site-packages`
3. Only directories with valid structure are used

## Implementation Details

### Site-packages Detection

Serpen automatically detects site-packages directories for different Python versions:

- **Single version**: `venv/lib/python3.11/site-packages`
- **Multiple versions**: All `python*` directories are scanned
- **Windows**: Uses `Lib/site-packages` structure

### Package Name Extraction

From site-packages directories, Serpen identifies packages by:

- **Directory names**: Package directories (e.g., `requests/`)
- **Module files**: Single-file modules (e.g., `six.py`)
- **Filtering**: Excludes system files (`__pycache__`, `*.dist-info`, etc.)

### Submodule Handling

For import `requests.auth`, Serpen:

1. Checks if `requests.auth` exists as a package
2. If not found, checks if `requests` (root module) exists in VIRTUAL_ENV
3. If `requests` is found → classifies `requests.auth` as `ThirdParty`

## Module Shadowing and Priority

### Resolution Priority

When modules with the same name exist in multiple locations, Serpen follows
Python's import resolution order:

1. **First-party modules** (from `src` directories and `PYTHONPATH`) take
   **highest priority**
2. **Virtual environment packages** (from `VIRTUAL_ENV`) take **lower priority**
3. **Standard library modules** take **lowest priority**

### Shadowing Examples

#### Local Module Shadows Virtual Environment Package

```python
# If you have both:
# - src/requests.py (local module)
# - venv/lib/python3.11/site-packages/requests/ (installed package)

import requests  # → Resolves to src/requests.py (FirstParty)
```

Serpen correctly classifies `requests` as `FirstParty` because local modules
take precedence.

#### PYTHONPATH Module Shadows Virtual Environment Package

```python
# If you have both:
# - /my/pythonpath/numpy.py (PYTHONPATH module)
# - venv/lib/python3.11/site-packages/numpy/ (installed package)

import numpy  # → Resolves to /my/pythonpath/numpy.py (FirstParty)
```

Modules from `PYTHONPATH` also take precedence over virtual environment
packages.

#### Submodule Shadowing

```python
# If src/requests.py shadows venv/requests/:
import requests.auth  # → Still classified as FirstParty
```

When a top-level module is shadowed, its submodules are also classified using
the shadowing module's type.

#### No Shadowing

```python
# If only venv/lib/python3.11/site-packages/flask/ exists:
import flask  # → Resolves to virtual environment (ThirdParty)
```

Virtual environment packages are classified as `ThirdParty` when no local
shadowing occurs.

### Testing Shadowing Behavior

The test suite includes comprehensive shadowing scenarios:

```rust
#[test]
fn test_module_shadowing_priority() {
    // Local src/requests.py shadows venv/requests/
    assert_eq!(resolver.classify_import("requests"), ImportType::FirstParty);

    // PYTHONPATH numpy.py shadows venv/numpy/
    assert_eq!(resolver.classify_import("numpy"), ImportType::FirstParty);

    // No shadowing: venv/flask/ remains third-party
    assert_eq!(resolver.classify_import("flask"), ImportType::ThirdParty);
}
```

This ensures that Serpen's bundling behavior matches Python's actual import
resolution.

## Configuration Integration

### Known Third-party

VIRTUAL_ENV works alongside explicit configuration:

```toml
[tool.serpen]
known_third_party = ["custom_package"]
```

Modules in `known_third_party` are always classified as third-party, regardless of VIRTUAL_ENV.

### Known First-party

```toml
[tool.serpen]
known_first_party = ["my_local_package"]
```

Explicit first-party configuration takes precedence over VIRTUAL_ENV detection.

## Testing Support

### VirtualEnvGuard

For testing, Serpen provides `VirtualEnvGuard` for safe environment manipulation:

```rust
use serpen::resolver::VirtualEnvGuard;

// Set VIRTUAL_ENV for testing
let _guard = VirtualEnvGuard::new("/path/to/test/venv");

// Clear VIRTUAL_ENV for testing
let _guard = VirtualEnvGuard::unset();
// Automatically restored when guard is dropped
```

### Override Methods

Test-specific resolver creation with VIRTUAL_ENV override:

```rust
let resolver = ModuleResolver::new_with_virtualenv(config, Some("/test/venv"))?;
let resolver = ModuleResolver::new_with_overrides(config, pythonpath, virtualenv)?;
```

## Insights from Reference Implementations

Based on analysis of virtual environment handling in established Python tooling repositories (Ruff and Pyre), several patterns and improvements have been identified that could enhance Serpen's virtual environment support.

### Integration Considerations

When implementing these enhancements:

1. **Backward Compatibility**: Ensure existing virtual environment detection continues to work
2. **Configuration**: Allow users to enable/disable advanced features
3. **Performance**: Measure impact of new features on startup time
4. **Testing**: Add comprehensive test coverage for new functionality
5. **Documentation**: Update user documentation with new capabilities

These insights from established Python tooling provide a roadmap for evolving Serpen's virtual environment support to handle modern Python development workflows while maintaining reliability and performance.

## Advanced Virtual Environment Patterns

This section documents additional advanced patterns discovered from deeper analysis of production Python tooling implementations.

### IDE Integration Best Practices

Based on Pyre's VSCode extension implementation, here are proven patterns for IDE integration:

#### Environment Path Resolution for IDEs

```typescript
// Adaptive environment path resolution
async function findPyreCommand(envPath: EnvironmentPath): Promise<string | undefined> {
    // Handle default system Python
    if (envPath.id === 'DEFAULT_PYTHON') {
        return 'pyre';
    }
    
    const path = envPath.path;
    const stat = statSync(path);
    
    // Try different executable locations based on path type
    const pyrePath = stat.isFile()
        ? join(dirname(envPath.path), 'pyre')     // Executable path
        : stat.isDirectory()
            ? join(path, 'bin', 'pyre')           // Environment directory
            : undefined;
    
    // Validate executable exists and is accessible
    if (pyrePath && existsSync(pyrePath) && statSync(pyrePath).isFile()) {
        return pyrePath;
    }
    
    // Fallback to system PATH
    return await which('pyre', { nothrow: true });
}
```

**Key Patterns**:

- **Multi-mode detection**: Handle both executable paths and environment directories
- **Graceful fallback**: Always provide fallback to system PATH
- **Validation**: Verify executable exists and is accessible
- **Cross-platform**: Handle different executable extensions and paths

#### Dynamic Environment Switching

```typescript
// Handle environment changes in real-time
envListener = pythonExtension.exports.environments.onDidChangeActiveEnvironmentPath(async (e) => {
    // Clean up previous state
    state?.languageClient?.stop();
    state?.configListener.then((listener) => listener.dispose());
    state = undefined;
    
    // Initialize with new environment
    const pyrePath = await findPyreCommand(e);
    if (pyrePath) {
        state = createLanguageClient(pyrePath);
    }
});
```

**Key Patterns**:

- **State cleanup**: Properly dispose of previous resources
- **Asynchronous handling**: Handle environment changes without blocking
- **Error resilience**: Continue operation even if new environment fails

### Development Workflow Integration

#### Virtual Environment Lifecycle Management

Based on patterns observed in production tooling, here are strategies for integrating with development workflows:

```rust
// Automatic environment detection during development
pub struct DevelopmentEnvironmentDetector {
    project_root: PathBuf,
    cache: HashMap<PathBuf, CachedEnvironmentInfo>,
}

impl DevelopmentEnvironmentDetector {
    pub fn detect_project_environments(&self) -> Vec<DetectedEnvironment> {
        let mut environments = Vec::new();

        // Check for common virtual environment patterns
        let venv_patterns = [
            ".venv",       // Modern Python standard
            "venv",        // Traditional name
            "env",         // Alternative name
            ".virtualenv", // Legacy virtualenv
            "virtualenv",  // Alternative legacy
        ];

        for pattern in &venv_patterns {
            let env_path = self.project_root.join(pattern);
            if self.is_valid_virtual_environment(&env_path) {
                environments.push(DetectedEnvironment {
                    path: env_path,
                    env_type: EnvironmentType::Local,
                    priority: self.get_environment_priority(pattern),
                    metadata: self.extract_environment_metadata(&env_path),
                });
            }
        }

        // Check for tool-specific environments
        environments.extend(self.detect_tool_specific_environments());

        // Sort by priority (highest first)
        environments.sort_by(|a, b| b.priority.cmp(&a.priority));
        environments
    }

    fn detect_tool_specific_environments(&self) -> Vec<DetectedEnvironment> {
        let mut environments = Vec::new();

        // Poetry environment detection
        if self.project_root.join("pyproject.toml").exists() {
            if let Some(poetry_env) = self.detect_poetry_environment() {
                environments.push(poetry_env);
            }
        }

        // Pipenv environment detection
        if self.project_root.join("Pipfile").exists() {
            if let Some(pipenv_env) = self.detect_pipenv_environment() {
                environments.push(pipenv_env);
            }
        }

        // Conda environment detection
        if let Some(conda_env) = self.detect_conda_environment() {
            environments.push(conda_env);
        }

        environments
    }
}
```

#### Environment-Aware Configuration

```rust
// Configuration that adapts to virtual environment context
#[derive(Debug, Clone)]
pub struct EnvironmentAwareConfig {
    base_config: SerpenConfig,
    environment_overrides: HashMap<PathBuf, ConfigOverrides>,
    auto_detect_environments: bool,
    environment_priority_order: Vec<EnvironmentType>,
}

impl EnvironmentAwareConfig {
    pub fn resolve_for_environment(&self, env_path: Option<&Path>) -> SerpenConfig {
        let mut config = self.base_config.clone();

        // Apply environment-specific overrides
        if let Some(env_path) = env_path {
            if let Some(overrides) = self.environment_overrides.get(env_path) {
                config.apply_overrides(overrides);
            }
        }

        // Auto-detect and configure based on environment type
        if self.auto_detect_environments {
            if let Some(env_info) = env_path.and_then(|p| self.detect_environment_info(p)) {
                config.apply_environment_defaults(&env_info);
            }
        }

        config
    }

    fn apply_environment_defaults(&mut self, env_info: &EnvironmentInfo) {
        match env_info.env_type {
            EnvironmentType::Poetry => {
                // Poetry-specific defaults
                self.known_third_party
                    .extend(env_info.detected_packages.clone());
                self.src_paths.push("src".into());
            }
            EnvironmentType::Pipenv => {
                // Pipenv-specific defaults
                self.follow_imports = ImportFollowMode::Normal;
            }
            EnvironmentType::Conda => {
                // Conda-specific defaults
                self.include_system_site_packages = true;
            }
            _ => {}
        }
    }
}
```

### Advanced Package Resolution Strategies

#### Multi-Source Package Discovery

```rust
// Comprehensive package discovery across multiple sources
pub struct MultiSourcePackageResolver {
    sources: Vec<Box<dyn PackageSource>>,
    cache: PackageCache,
    resolution_strategy: ResolutionStrategy,
}

trait PackageSource {
    fn discover_packages(&self) -> Result<Vec<PackageInfo>, PackageError>;
    fn resolve_package(&self, name: &str) -> Result<Option<PackageLocation>, PackageError>;
    fn source_priority(&self) -> u8;
    fn source_type(&self) -> SourceType;
}

// Virtual environment package source
struct VirtualEnvPackageSource {
    env_path: PathBuf,
    site_packages_paths: Vec<PathBuf>,
}

impl PackageSource for VirtualEnvPackageSource {
    fn discover_packages(&self) -> Result<Vec<PackageInfo>, PackageError> {
        let mut packages = Vec::new();

        for site_packages in &self.site_packages_paths {
            packages.extend(self.scan_site_packages(site_packages)?);
        }

        // Deduplicate by package name, keeping highest version
        packages.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| b.version.cmp(&a.version)));
        packages.dedup_by(|a, b| a.name == b.name);

        Ok(packages)
    }

    fn resolve_package(&self, name: &str) -> Result<Option<PackageLocation>, PackageError> {
        // Fast path: check cache first
        if let Some(cached) = self.cache.get(name) {
            return Ok(Some(cached));
        }

        // Search in all site-packages directories
        for site_packages in &self.site_packages_paths {
            if let Some(location) = self.find_package_in_directory(site_packages, name)? {
                self.cache.insert(name.to_string(), location.clone());
                return Ok(Some(location));
            }
        }

        Ok(None)
    }
}

// System package source (fallback)
struct SystemPackageSource {
    system_paths: Vec<PathBuf>,
}

// Conda package source
struct CondaPackageSource {
    conda_prefix: PathBuf,
    environment_name: Option<String>,
}
```

#### Intelligent Package Caching

```rust
// Smart caching system that handles virtual environment changes
pub struct IntelligentPackageCache {
    cache: HashMap<CacheKey, CacheEntry>,
    environment_signatures: HashMap<PathBuf, EnvironmentSignature>,
    max_cache_size: usize,
    ttl: Duration,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    environment_path: PathBuf,
    package_name: String,
    environment_signature: u64, // Hash of environment state
}

#[derive(Debug, Clone)]
struct CacheEntry {
    package_location: PackageLocation,
    cached_at: SystemTime,
    access_count: usize,
    last_accessed: SystemTime,
}

impl IntelligentPackageCache {
    pub fn get_package(&mut self, env_path: &Path, package_name: &str) -> Option<&PackageLocation> {
        // Check if environment signature has changed
        let current_signature = self.compute_environment_signature(env_path);
        let cached_signature = self.environment_signatures.get(env_path);

        if cached_signature != Some(&current_signature) {
            // Environment changed, invalidate related cache entries
            self.invalidate_environment_cache(env_path);
            self.environment_signatures
                .insert(env_path.to_path_buf(), current_signature);
            return None;
        }

        // Look up in cache
        let cache_key = CacheKey {
            environment_path: env_path.to_path_buf(),
            package_name: package_name.to_string(),
            environment_signature: current_signature.hash,
        };

        if let Some(entry) = self.cache.get_mut(&cache_key) {
            // Check TTL
            if entry.cached_at.elapsed().unwrap_or_default() > self.ttl {
                self.cache.remove(&cache_key);
                return None;
            }

            // Update access statistics
            entry.access_count += 1;
            entry.last_accessed = SystemTime::now();

            Some(&entry.package_location)
        } else {
            None
        }
    }

    fn compute_environment_signature(&self, env_path: &Path) -> EnvironmentSignature {
        let mut hasher = DefaultHasher::new();

        // Hash pyvenv.cfg modification time
        if let Ok(metadata) = fs::metadata(env_path.join("pyvenv.cfg")) {
            if let Ok(modified) = metadata.modified() {
                hasher.write_u64(
                    modified
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                );
            }
        }

        // Hash site-packages directory modification times
        for site_packages in self.get_site_packages_paths(env_path) {
            if let Ok(metadata) = fs::metadata(&site_packages) {
                if let Ok(modified) = metadata.modified() {
                    hasher.write_u64(
                        modified
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                    );
                }
            }
        }

        EnvironmentSignature {
            hash: hasher.finish(),
            computed_at: SystemTime::now(),
        }
    }
}
```

### Testing Framework Enhancements

#### Comprehensive Virtual Environment Testing

```rust
// Advanced testing framework for virtual environment scenarios
pub struct VirtualEnvironmentTestFramework {
    test_environments: HashMap<String, TestEnvironmentBuilder>,
    temp_dir: TempDir,
}

impl VirtualEnvironmentTestFramework {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            test_environments: HashMap::new(),
            temp_dir: TempDir::new()?,
        })
    }

    pub fn create_test_environment(&mut self, name: &str) -> &mut TestEnvironmentBuilder {
        let builder = TestEnvironmentBuilder::new(self.temp_dir.path().join(name));
        self.test_environments.insert(name.to_string(), builder);
        self.test_environments.get_mut(name).unwrap()
    }

    pub fn with_realistic_packages<F>(&mut self, name: &str, setup: F) -> &mut Self
    where
        F: FnOnce(&mut TestEnvironmentBuilder),
    {
        let builder = self.create_test_environment(name);
        setup(builder);
        self
    }
}

pub struct TestEnvironmentBuilder {
    env_path: PathBuf,
    packages: Vec<TestPackage>,
    python_version: (u8, u8),
    pyvenv_config: HashMap<String, String>,
    extends_environment: Option<PathBuf>,
}

impl TestEnvironmentBuilder {
    pub fn with_package(mut self, name: &str, version: &str) -> Self {
        self.packages.push(TestPackage {
            name: name.to_string(),
            version: version.to_string(),
            files: vec![format!("{name}/__init__.py")],
            dependencies: vec![],
        });
        self
    }

    pub fn with_package_structure<F>(mut self, name: &str, structure_fn: F) -> Self
    where
        F: FnOnce(&mut TestPackageBuilder),
    {
        let mut builder = TestPackageBuilder::new(name);
        structure_fn(&mut builder);
        self.packages.push(builder.build());
        self
    }

    pub fn with_ephemeral_parent(mut self, parent_path: PathBuf) -> Self {
        self.extends_environment = Some(parent_path);
        self.pyvenv_config.insert(
            "extends-environment".to_string(),
            parent_path.to_string_lossy().to_string(),
        );
        self.pyvenv_config
            .insert("uv".to_string(), "0.7.6".to_string());
        self
    }

    pub fn build(self) -> io::Result<TestVirtualEnvironment> {
        // Create directory structure
        fs::create_dir_all(&self.env_path)?;

        // Create site-packages
        let site_packages = if cfg!(target_os = "windows") {
            self.env_path.join("Lib").join("site-packages")
        } else {
            self.env_path
                .join("lib")
                .join(format!(
                    "python{}.{}",
                    self.python_version.0, self.python_version.1
                ))
                .join("site-packages")
        };
        fs::create_dir_all(&site_packages)?;

        // Install test packages
        for package in &self.packages {
            package.install_to(&site_packages)?;
        }

        // Create pyvenv.cfg
        let mut pyvenv_content = format!(
            "home = /usr/bin\n\
             include-system-site-packages = false\n\
             version = {}.{}\n",
            self.python_version.0, self.python_version.1
        );

        for (key, value) in &self.pyvenv_config {
            pyvenv_content.push_str(&format!("{key} = {value}\n"));
        }

        fs::write(self.env_path.join("pyvenv.cfg"), pyvenv_content)?;

        Ok(TestVirtualEnvironment {
            path: self.env_path,
            packages: self.packages,
        })
    }
}

// Realistic package testing scenarios
#[cfg(test)]
mod realistic_tests {
    use super::*;

    #[test]
    fn test_django_project_environment() {
        let mut framework = VirtualEnvironmentTestFramework::new().unwrap();

        let env = framework
            .with_realistic_packages("django_env", |builder| {
                builder
                    .with_package_structure("django", |pkg| {
                        pkg.with_version("4.2.0")
                            .with_submodules(&[
                                "contrib/admin",
                                "core/management",
                                "db/models",
                                "http",
                                "urls",
                            ])
                            .with_dependencies(&["sqlparse", "asgiref"])
                    })
                    .with_package("sqlparse", "0.4.4")
                    .with_package("asgiref", "3.7.2")
                    .with_package_structure("requests", |pkg| {
                        pkg.with_version("2.31.0")
                            .with_submodules(&["auth", "adapters", "models"])
                    });
            })
            .create_test_environment("django_env")
            .build()
            .unwrap();

        // Test package resolution
        let resolver = VirtualEnvironmentResolver::new(&env.path).unwrap();

        assert!(resolver.resolve_package("django").is_some());
        assert!(resolver.resolve_package("django.contrib.admin").is_some());
        assert!(resolver.resolve_package("requests.auth").is_some());
        assert!(resolver.resolve_package("nonexistent").is_none());
    }

    #[test]
    fn test_data_science_environment() {
        let mut framework = VirtualEnvironmentTestFramework::new().unwrap();

        framework.with_realistic_packages("datascience_env", |builder| {
            builder
                .with_package_structure("numpy", |pkg| {
                    pkg.with_version("1.24.3")
                        .with_c_extensions(&["core/_multiarray_umath"])
                })
                .with_package_structure("pandas", |pkg| {
                    pkg.with_version("2.0.3")
                        .with_submodules(&["core", "io", "plotting"])
                        .with_dependencies(&["numpy", "python-dateutil", "pytz"])
                })
                .with_package("matplotlib", "3.7.1")
                .with_package("scipy", "1.11.1");
        });

        // Test complex dependency resolution
        // ... test implementation
    }
}
```

These additional patterns provide:

1. **Development Workflow Integration**: Automatic detection of different virtual environment types and tool-specific configurations
2. **Advanced Package Resolution**: Multi-source package discovery with intelligent caching
3. **Comprehensive Testing**: Realistic test environments that simulate complex real-world scenarios

These enhancements build upon the existing virtual environment support to provide a robust, production-ready system that can handle the complexity of modern Python development workflows.

## Benefits

1. **Improved Accuracy**: Better distinction between third-party and standard library modules
2. **Virtual Environment Aware**: Automatically adapts to different virtual environments
3. **Cross-platform**: Works on Windows, macOS, and Linux
4. **Multiple Python Versions**: Handles virtual environments with different Python versions
5. **Zero Configuration**: Works automatically when virtual environments are activated

## Compatibility

- **Python Versions**: Supports Python 3.6+ virtual environments
- **Virtual Environment Tools**: Compatible with `venv`, `virtualenv`, `conda`, etc.
- **Platforms**: Windows, macOS, Linux
- **Path Formats**: Handles both absolute and relative paths

## Performance Considerations

- Site-packages scanning is performed once during resolver initialization
- Package registry is cached for subsequent import classifications
- Only existing directories are scanned to avoid filesystem errors
- Minimal overhead when VIRTUAL_ENV is not set

## Limitations

1. **Static Analysis**: Only detects packages installed at analysis time
2. **File System Access**: Requires read access to virtual environment directories
3. **No Dynamic Imports**: Cannot detect packages imported dynamically at runtime
4. **Package Naming**: Relies on standard package installation conventions

## Examples

### Basic Virtual Environment

```bash
# Setup
python -m venv myproject_venv
source myproject_venv/bin/activate
pip install requests numpy

# Analysis
serpen --entry app.py --output bundle.py
```

In this case:

- `import requests` → `ThirdParty` (found in venv)
- `import numpy` → `ThirdParty` (found in venv)
- `import os` → `StandardLibrary`
- `import mymodule` → `FirstParty` (found in src directories)

### Multiple Python Versions

```bash
# Virtual environment with multiple Python versions
ls venv/lib/
# python3.10/ python3.11/

serpen --entry app.py --output bundle.py
```

Serpen will scan both `python3.10/site-packages` and `python3.11/site-packages`.

### Complex Project Structure

```
project/
├── src/
│   ├── myapp/
│   └── utils/
├── venv/
│   └── lib/python3.11/site-packages/
│       ├── requests/
│       └── numpy/
└── app.py
```

Import classification:

- `import myapp` → `FirstParty` (found in src/)
- `import utils` → `FirstParty` (found in src/)
- `import requests` → `ThirdParty` (found in venv/)
- `import numpy.array` → `ThirdParty` (numpy found in venv/)

## Related Documentation

- [PYTHONPATH Support](./pythonpath_support.md) - First-party module discovery
- [Import Resolution Analysis](./serpen_import_resolution_analysis.md) - Overall import resolution strategy
- [Configuration Guide](../README.md) - General configuration options
