use ruff_text_size::TextRange;

use crate::resolver::ImportType;


/// Basic import information for compatibility
#[derive(Debug, Clone)]
pub struct ImportInfo {
    /// The module being imported
    pub module_name: String,
    /// Names imported from the module with their aliases (name, alias)
    pub imported_names: Vec<(String, Option<String>)>,
    /// Type of import (stdlib, first-party, third-party)
    pub import_type: ImportType,
    /// Line number where import occurs
    pub line_number: usize,
}
