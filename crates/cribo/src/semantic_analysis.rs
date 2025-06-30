use ruff_text_size::TextRange;

use crate::resolver::ImportType;

/// Represents the execution context of code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionContext {
    /// Code that executes at module import time
    ModuleLevel,
    /// Code inside function/method bodies (deferred execution)
    FunctionBody,
    /// Code inside class bodies (executes during class definition)
    ClassBody,
    /// Code inside type annotations (may not execute at runtime)
    TypeAnnotation,
    /// Code inside if TYPE_CHECKING blocks (typing-only)
    TypeCheckingBlock,
}

impl ExecutionContext {
    /// Returns true if this context requires runtime availability of imports
    pub fn requires_runtime(&self) -> bool {
        matches!(self, Self::ModuleLevel | Self::ClassBody)
    }

    /// Returns true if this context is deferred (not executed at import time)
    pub fn is_deferred(&self) -> bool {
        matches!(
            self,
            Self::FunctionBody | Self::TypeAnnotation | Self::TypeCheckingBlock
        )
    }
}

/// Tracks how an import is used in the code
#[derive(Debug, Clone)]
pub struct ImportUsage {
    /// The name being used (might be aliased)
    pub name: String,
    /// The original import name
    pub import_name: String,
    /// Where this usage occurs
    pub usage_context: ExecutionContext,
    /// The location of the usage
    pub location: TextRange,
}

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
