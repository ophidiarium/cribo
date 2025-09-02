//! Enhanced import alias tracking for proper bundling
//!
//! This module provides structures and utilities to track the relationship
//! between imported names and their aliases, which is crucial for correctly
//! rewriting imports in the bundled output.

use crate::{resolver::ModuleId, types::FxIndexMap};

/// Enhanced information about a from-import that tracks both the original name and alias
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnhancedFromImport {
    /// The module being imported from (e.g., "requests.compat" in `from requests.compat import
    /// ...`)
    pub module: String,
    /// The original name being imported (e.g., "`JSONDecodeError`")
    pub original_name: String,
    /// The local alias used, if any (e.g., "`CompatJSONDecodeError`" in `... as
    /// CompatJSONDecodeError`)
    pub local_alias: Option<String>,
}

/// Tracks import alias information across modules
#[derive(Debug, Default)]
pub struct ImportAliasTracker {
    /// Maps (`module_id`, `local_name`) to the enhanced import information
    imports: FxIndexMap<(ModuleId, String), EnhancedFromImport>,
}

impl ImportAliasTracker {
    /// Create a new import alias tracker
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a from-import with potential alias
    pub fn register_from_import(
        &mut self,
        module_id: ModuleId,
        module: String,
        original_name: String,
        local_alias: Option<String>,
    ) {
        let import_info = EnhancedFromImport {
            module,
            original_name: original_name.clone(),
            local_alias: local_alias.clone(),
        };

        let local_name = local_alias.unwrap_or(original_name);
        self.imports.insert((module_id, local_name), import_info);
    }
}
