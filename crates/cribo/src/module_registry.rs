//! Module registry for tracking module information during bundling
//!
//! The ModuleRegistry is the single source of truth for module identity
//! throughout the bundling process. It maintains mappings between module IDs,
//! canonical names, and file paths.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use indexmap::IndexMap;
use ruff_python_ast::{AtomicNodeIndex, ModModule};
use rustc_hash::FxHasher;

use crate::cribo_graph::{ItemId, ModuleId};

/// Type alias for FxHasher-based IndexMap
type FxIndexMap<K, V> = IndexMap<K, V, std::hash::BuildHasherDefault<FxHasher>>;

/// Complete information about a module needed during bundling
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// Unique identifier for this module
    pub id: ModuleId,
    /// Canonical import name (e.g., "utils.helpers")
    pub canonical_name: String,
    /// Resolved file path on disk
    pub resolved_path: PathBuf,
    /// Shared reference to the original source code
    pub original_source: Arc<String>,
    /// SHA-256 hash of the source content (hex-encoded)
    pub content_hash: String,
    /// Original parsed AST (before any transformations)
    pub original_ast: ModModule,
    /// Whether this module will be wrapped in an init function
    pub is_wrapper: bool,
    /// Mapping from ItemId to AST node index for precise transformations
    pub item_to_node: FxIndexMap<ItemId, AtomicNodeIndex>,
    /// Reverse mapping for quick lookups
    pub node_to_item: FxIndexMap<AtomicNodeIndex, ItemId>,
}

/// Central registry for module information
/// This is the single source of truth for module identity throughout the bundling process
#[derive(Debug, Clone)]
pub struct ModuleRegistry {
    /// Map from ModuleId to complete module information
    modules: FxIndexMap<ModuleId, ModuleInfo>,
    /// Map from canonical name to ModuleId for fast lookups
    name_to_id: FxIndexMap<String, ModuleId>,
    /// Map from resolved path to ModuleId for fast lookups
    path_to_id: FxIndexMap<PathBuf, ModuleId>,
    /// Map from content hash to ModuleId for deduplication
    hash_to_id: FxIndexMap<String, ModuleId>,
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleRegistry {
    /// Create a new empty module registry
    pub fn new() -> Self {
        Self {
            modules: FxIndexMap::default(),
            name_to_id: FxIndexMap::default(),
            path_to_id: FxIndexMap::default(),
            hash_to_id: FxIndexMap::default(),
        }
    }

    /// Add a module to the registry
    /// Returns the ModuleId - either the new one or existing one if content hash matches
    pub fn add_module(&mut self, info: ModuleInfo) -> ModuleId {
        let id = info.id;
        let name = info.canonical_name.clone();
        let path = info.resolved_path.clone();
        let hash = info.content_hash.clone();

        // Check if we already have a module with this content hash
        if let Some(&existing_id) = self.hash_to_id.get(&hash) {
            // Content already exists - validate it's the same module or compatible
            let existing = self
                .modules
                .get(&existing_id)
                .expect("Hash exists but module not found");

            log::debug!(
                "Module with hash {} already registered as {:?} ({}), requested registration as \
                 {:?} ({})",
                &hash[..8], // First 8 chars of hash for readability
                existing_id,
                existing.canonical_name,
                id,
                name
            );

            // Update name mapping if this is a new import path for the same content
            if !self.name_to_id.contains_key(&name) {
                self.name_to_id.insert(name, existing_id);
            }

            // Update path mapping if this is a new file path for the same content
            if !self.path_to_id.contains_key(&path) {
                self.path_to_id.insert(path, existing_id);
            }

            return existing_id;
        }

        // Check if module ID already exists with different content
        if let Some(existing) = self.modules.get(&id) {
            if existing.content_hash != hash {
                let existing_preview = if existing.content_hash.len() >= 8 {
                    &existing.content_hash[..8]
                } else {
                    &existing.content_hash
                };
                let new_preview = if hash.len() >= 8 { &hash[..8] } else { &hash };
                panic!(
                    "Attempting to register module {id:?} with different content. Existing hash: \
                     {existing_preview}, New hash: {new_preview}"
                );
            }
            return id; // Module already registered with same content
        }

        // Register new module
        self.name_to_id.insert(name, id);
        self.path_to_id.insert(path, id);
        self.hash_to_id.insert(hash, id);
        self.modules.insert(id, info);

        id
    }

    /// Get module info by ID
    pub fn get_by_id(&self, id: &ModuleId) -> Option<&ModuleInfo> {
        self.modules.get(id)
    }

    /// Get module ID by canonical name
    pub fn get_id_by_name(&self, name: &str) -> Option<ModuleId> {
        self.name_to_id.get(name).copied()
    }

    /// Get module ID by resolved path
    pub fn get_id_by_path(&self, path: &Path) -> Option<ModuleId> {
        self.path_to_id.get(path).copied()
    }

    /// Get module info by ID (alias for get_by_id for backwards compatibility)
    pub fn get_module_by_id(&self, id: ModuleId) -> Option<&ModuleInfo> {
        self.modules.get(&id)
    }

    /// Get mutable module info by ID
    pub fn get_module_mut(&mut self, id: ModuleId) -> Option<&mut ModuleInfo> {
        self.modules.get_mut(&id)
    }

    /// Iterate over all modules
    pub fn iter(&self) -> impl Iterator<Item = (&ModuleId, &ModuleInfo)> {
        self.modules.iter()
    }

    /// Get the canonical name for a module by its ID
    pub fn get_name_by_id(&self, id: ModuleId) -> Option<&str> {
        self.modules
            .get(&id)
            .map(|info| info.canonical_name.as_str())
    }

    /// Check if a module with the given name exists in the registry
    pub fn has_module(&self, name: &str) -> bool {
        self.name_to_id.contains_key(name)
    }

    /// Get total number of modules in the registry
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Get all module names
    pub fn module_names(&self) -> impl Iterator<Item = &str> {
        self.name_to_id.keys().map(|s| s.as_str())
    }

    /// Get all module IDs
    pub fn module_ids(&self) -> impl Iterator<Item = &ModuleId> {
        self.modules.keys()
    }

    /// Clear the registry
    pub fn clear(&mut self) {
        self.modules.clear();
        self.name_to_id.clear();
        self.path_to_id.clear();
        self.hash_to_id.clear();
    }

    /// Get module ID by content hash
    pub fn get_id_by_hash(&self, hash: &str) -> Option<ModuleId> {
        self.hash_to_id.get(hash).copied()
    }

    /// Check if a module with the given content hash exists
    pub fn has_hash(&self, hash: &str) -> bool {
        self.hash_to_id.contains_key(hash)
    }

    /// Get the content hash for a module
    pub fn get_hash_by_id(&self, id: ModuleId) -> Option<&str> {
        self.modules.get(&id).map(|info| info.content_hash.as_str())
    }

    /// Generate a synthetic module name for a module by its ID
    /// Uses content hash to ensure deterministic output
    pub fn get_synthetic_name_by_id(&self, id: ModuleId) -> Option<String> {
        self.modules.get(&id).map(|info| {
            let module_name_escaped =
                Self::sanitize_module_name_for_identifier(&info.canonical_name);
            // Use first 6 characters of content hash for readability
            let short_hash = if info.content_hash.len() >= 6 {
                &info.content_hash[..6]
            } else {
                &info.content_hash
            };
            format!("__cribo_{short_hash}_{module_name_escaped}")
        })
    }

    /// Generate a synthetic module name for a module by its canonical name
    /// Uses content hash to ensure deterministic output
    pub fn get_synthetic_name_by_name(&self, module_name: &str) -> Option<String> {
        self.get_id_by_name(module_name)
            .and_then(|id| self.get_synthetic_name_by_id(id))
    }

    /// Sanitize module name for use as Python identifier
    pub fn sanitize_module_name_for_identifier(module_name: &str) -> String {
        module_name.replace(['.', '-'], "_")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_registry_basic_operations() {
        let mut registry = ModuleRegistry::new();
        assert!(registry.is_empty());

        // Create a test module info
        let module_id = ModuleId::new(1);
        let content_hash = "a1b2c3d4e5f6".to_string();
        let module_info = ModuleInfo {
            id: module_id,
            canonical_name: "test.module".to_string(),
            resolved_path: PathBuf::from("/path/to/test/module.py"),
            original_source: Arc::new("# test module".to_string()),
            content_hash: content_hash.clone(),
            original_ast: ModModule {
                body: Vec::new(),
                range: ruff_text_size::TextRange::default(),
                node_index: AtomicNodeIndex::dummy(),
            },
            is_wrapper: false,
            item_to_node: FxIndexMap::default(),
            node_to_item: FxIndexMap::default(),
        };

        // Add module
        let returned_id = registry.add_module(module_info.clone());
        assert_eq!(returned_id, module_id);
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());

        // Test lookups
        assert_eq!(registry.get_id_by_name("test.module"), Some(module_id));
        assert_eq!(
            registry.get_id_by_path(Path::new("/path/to/test/module.py")),
            Some(module_id)
        );
        assert_eq!(registry.get_id_by_hash(&content_hash), Some(module_id));
        assert!(registry.has_module("test.module"));
        assert!(registry.has_hash(&content_hash));
        assert_eq!(registry.get_name_by_id(module_id), Some("test.module"));
        assert_eq!(
            registry.get_hash_by_id(module_id),
            Some(content_hash.as_str())
        );

        // Test get_by_id
        let retrieved = registry.get_by_id(&module_id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().canonical_name, "test.module");

        // Test iteration
        let names: Vec<&str> = registry.module_names().collect();
        assert_eq!(names, vec!["test.module"]);

        // Clear registry
        registry.clear();
        assert!(registry.is_empty());
    }

    #[test]
    #[should_panic(expected = "different content")]
    fn test_module_registry_conflict_detection() {
        let mut registry = ModuleRegistry::new();

        let module_id = ModuleId::new(1);
        let module_info1 = ModuleInfo {
            id: module_id,
            canonical_name: "test.module".to_string(),
            resolved_path: PathBuf::from("/path/to/test/module.py"),
            original_source: Arc::new("content1".to_string()),
            content_hash: "hash1".to_string(),
            original_ast: ModModule {
                body: Vec::new(),
                range: ruff_text_size::TextRange::default(),
                node_index: AtomicNodeIndex::dummy(),
            },
            is_wrapper: false,
            item_to_node: FxIndexMap::default(),
            node_to_item: FxIndexMap::default(),
        };

        // Add first module
        registry.add_module(module_info1);

        // Try to add module with same ID but different content hash
        let module_info2 = ModuleInfo {
            id: module_id,
            canonical_name: "test.module".to_string(),
            resolved_path: PathBuf::from("/path/to/test/module.py"),
            original_source: Arc::new("content2".to_string()),
            content_hash: "hash2".to_string(),
            original_ast: ModModule {
                body: Vec::new(),
                range: ruff_text_size::TextRange::default(),
                node_index: AtomicNodeIndex::dummy(),
            },
            is_wrapper: false,
            item_to_node: FxIndexMap::default(),
            node_to_item: FxIndexMap::default(),
        };

        registry.add_module(module_info2); // This should panic
    }

    #[test]
    fn test_module_registry_deduplication() {
        let mut registry = ModuleRegistry::new();

        // Create two modules with same content but different paths/names
        let content_hash = "samehash123".to_string();

        let module_info1 = ModuleInfo {
            id: ModuleId::new(1),
            canonical_name: "package1.utils".to_string(),
            resolved_path: PathBuf::from("/path/to/package1/utils.py"),
            original_source: Arc::new("# same content".to_string()),
            content_hash: content_hash.clone(),
            original_ast: ModModule {
                body: Vec::new(),
                range: ruff_text_size::TextRange::default(),
                node_index: AtomicNodeIndex::dummy(),
            },
            is_wrapper: false,
            item_to_node: FxIndexMap::default(),
            node_to_item: FxIndexMap::default(),
        };

        let module_info2 = ModuleInfo {
            id: ModuleId::new(2),
            canonical_name: "package2.helpers".to_string(),
            resolved_path: PathBuf::from("/path/to/package2/helpers.py"),
            original_source: Arc::new("# same content".to_string()),
            content_hash: content_hash.clone(),
            original_ast: ModModule {
                body: Vec::new(),
                range: ruff_text_size::TextRange::default(),
                node_index: AtomicNodeIndex::dummy(),
            },
            is_wrapper: false,
            item_to_node: FxIndexMap::default(),
            node_to_item: FxIndexMap::default(),
        };

        // Add first module
        let id1 = registry.add_module(module_info1);
        assert_eq!(id1, ModuleId::new(1));
        assert_eq!(registry.len(), 1);

        // Add second module with same content - should return first module's ID
        let id2 = registry.add_module(module_info2);
        assert_eq!(id2, ModuleId::new(1)); // Same as first!
        assert_eq!(registry.len(), 1); // Still only one module

        // Both names should resolve to the same module
        assert_eq!(
            registry.get_id_by_name("package1.utils"),
            Some(ModuleId::new(1))
        );
        assert_eq!(
            registry.get_id_by_name("package2.helpers"),
            Some(ModuleId::new(1))
        );

        // Both paths should resolve to the same module
        assert_eq!(
            registry.get_id_by_path(Path::new("/path/to/package1/utils.py")),
            Some(ModuleId::new(1))
        );
        assert_eq!(
            registry.get_id_by_path(Path::new("/path/to/package2/helpers.py")),
            Some(ModuleId::new(1))
        );

        // Hash lookup should work
        assert_eq!(
            registry.get_id_by_hash(&content_hash),
            Some(ModuleId::new(1))
        );
    }
}
