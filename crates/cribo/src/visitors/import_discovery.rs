//! Import discovery visitor that finds all imports in a Python module,
//! including those nested within functions, classes, and other scopes.
//! Also performs semantic analysis to determine import usage patterns.

use ruff_python_ast::{
    Expr, ExprAttribute, ExprCall, ExprName, ExprStringLiteral, Stmt, StmtImport, StmtImportFrom,
    visitor::{Visitor, walk_expr, walk_stmt},
};
use ruff_text_size::TextRange;

use crate::{
    resolver::ModuleId,
    semantic_bundler::SemanticBundler,
    types::{FxIndexMap, FxIndexSet},
};

/// Execution context for code - determines when code runs relative to module import
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionContext {
    /// Code at module level - executes when module is imported
    ModuleLevel,
    /// Inside a function body - executes when function is called
    FunctionBody,
    /// Inside a class body - executes when class is defined (at module import time)
    ClassBody,
    /// Inside a class method - executes when method is called
    ClassMethod { is_init: bool },
    /// Type annotation context - may not execute at runtime
    TypeAnnotation,
}

/// Usage information for an imported name
#[derive(Debug, Clone)]
pub struct ImportUsage {
    /// Where the name was used
    pub _location: TextRange,
    /// In what execution context
    pub _context: ExecutionContext,
    /// The actual name used (might be aliased)
    pub _name_used: String,
}

/// Type of import statement
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportType {
    /// import module
    Direct,
    /// from module import ...
    From,
    /// from . import ... (relative)
    Relative { level: u32 },
    /// `importlib.import_module("module`") with static string
    ImportlibStatic,
}

/// An import discovered during AST traversal
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredImport {
    /// The module being imported
    pub module_name: Option<String>,
    /// Names being imported (for from imports)
    pub names: Vec<(String, Option<String>)>, // (name, alias)
    /// Location where the import was found
    pub location: ImportLocation,
    /// Source range of the import statement
    pub range: TextRange,
    /// Import level for relative imports
    pub level: u32,
    /// Type of import
    pub import_type: ImportType,
    /// Execution contexts where this import is used
    pub execution_contexts: FxIndexSet<ExecutionContext>,
    /// Whether this import is used in a class __init__ method
    pub is_used_in_init: bool,
    /// Whether this import can be moved to function scope
    pub is_movable: bool,
    /// Whether this import is only used within `TYPE_CHECKING` blocks
    pub is_type_checking_only: bool,
    /// Package context for relative `ImportlibStatic` imports (e.g., "package" in
    /// `importlib.import_module(".submodule`", "package"))
    pub package_context: Option<String>,
}

/// Where an import was discovered in the AST
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportLocation {
    /// Import at module level
    Module,
    /// Import inside a function
    Function(String),
    /// Import inside a class definition
    Class(String),
    /// Import inside a method
    Method { class: String, method: String },
    /// Import inside a conditional block
    Conditional { depth: usize },
    /// Import inside other nested scope
    Nested(Vec<ScopeElement>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeElement {
    Function(String),
    Class(String),
    If,
    While,
    For,
    With,
    Try,
}

/// Visitor that discovers all imports in a Python module and analyzes their usage
pub struct ImportDiscoveryVisitor<'a> {
    /// All discovered imports
    imports: Vec<DiscoveredImport>,
    /// Current scope stack
    scope_stack: Vec<ScopeElement>,
    /// Stack of scope-local maps from imported names to their module sources
    imported_names_stack: Vec<FxIndexMap<String, String>>,
    /// Track usage of each imported name
    name_usage: FxIndexMap<String, Vec<ImportUsage>>,
    /// Optional reference to semantic bundler for enhanced analysis
    _semantic_bundler: Option<&'a SemanticBundler>,
    /// Current module ID if available
    _module_id: Option<ModuleId>,
    /// Current execution context
    current_context: ExecutionContext,
    /// Whether we're in a type checking block
    in_type_checking: bool,
    /// Track if we have importlib imported
    has_importlib: bool,
}

impl Default for ImportDiscoveryVisitor<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> ImportDiscoveryVisitor<'a> {
    /// Create a new import discovery visitor
    pub fn new() -> Self {
        log::debug!("Creating new ImportDiscoveryVisitor");
        Self {
            imports: Vec::new(),
            scope_stack: Vec::new(),
            imported_names_stack: vec![FxIndexMap::default()],
            name_usage: FxIndexMap::default(),
            _semantic_bundler: None,
            _module_id: None,
            current_context: ExecutionContext::ModuleLevel,
            in_type_checking: false,
            has_importlib: false,
        }
    }

    /// Create a new visitor with semantic bundler for enhanced analysis
    pub fn with_semantic_bundler(
        semantic_bundler: &'a SemanticBundler,
        module_id: ModuleId,
    ) -> Self {
        Self {
            imports: Vec::new(),
            scope_stack: Vec::new(),
            imported_names_stack: vec![FxIndexMap::default()],
            name_usage: FxIndexMap::default(),
            _semantic_bundler: Some(semantic_bundler),
            _module_id: Some(module_id),
            current_context: ExecutionContext::ModuleLevel,
            in_type_checking: false,
            has_importlib: false,
        }
    }

    #[inline]
    fn insert_imported_name(&mut self, name: String, module_key: String) {
        if let Some(top) = self.imported_names_stack.last_mut() {
            top.insert(name, module_key);
        }
    }

    #[inline]
    fn lookup_imported_name(&self, name: &str) -> Option<&String> {
        for scope in self.imported_names_stack.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    /// Get all discovered imports
    pub fn into_imports(mut self) -> Vec<DiscoveredImport> {
        // Post-process imports to determine movability based on usage
        for i in 0..self.imports.len() {
            let import = &self.imports[i];
            let is_movable = self.is_import_movable(import);
            self.imports[i].is_movable = is_movable;

            // An import is type-checking-only if it was imported in a TYPE_CHECKING block
            // AND is not used anywhere outside of TYPE_CHECKING blocks
            // We already set is_type_checking_only when the import was discovered
            // No need to update it here since we track usage contexts separately
        }
        self.imports
    }

    /// Get the current location based on scope stack
    fn current_location(&self) -> ImportLocation {
        if self.scope_stack.is_empty() {
            return ImportLocation::Module;
        }

        // Analyze the scope stack to determine location
        match &self.scope_stack[..] {
            [ScopeElement::Function(name)] => ImportLocation::Function(name.clone()),
            [ScopeElement::Class(name)] => ImportLocation::Class(name.clone()),
            [ScopeElement::Class(class), ScopeElement::Function(method)] => {
                ImportLocation::Method {
                    class: class.clone(),
                    method: method.clone(),
                }
            }
            _ => {
                // Check if we're in any conditional
                let conditional_depth = self
                    .scope_stack
                    .iter()
                    .filter(|s| {
                        matches!(
                            s,
                            ScopeElement::If | ScopeElement::While | ScopeElement::For
                        )
                    })
                    .count();

                if conditional_depth > 0 {
                    ImportLocation::Conditional {
                        depth: conditional_depth,
                    }
                } else {
                    ImportLocation::Nested(self.scope_stack.clone())
                }
            }
        }
    }

    /// Get current execution context based on scope stack
    fn get_current_execution_context(&self) -> ExecutionContext {
        if self.in_type_checking {
            return ExecutionContext::TypeAnnotation;
        }

        // Prefer innermost function scope when present.
        if let Some(last_fn_idx) = self
            .scope_stack
            .iter()
            .rposition(|s| matches!(s, ScopeElement::Function(_)))
        {
            // If the innermost function is the method itself (directly under a class)
            // and not a nested function, classify as ClassMethod. Otherwise FunctionBody.
            if last_fn_idx > 0
                && let (ScopeElement::Class(_), ScopeElement::Function(method_name)) = (
                    &self.scope_stack[last_fn_idx - 1],
                    &self.scope_stack[last_fn_idx],
                )
                && last_fn_idx == self.scope_stack.len() - 1
            {
                return ExecutionContext::ClassMethod {
                    is_init: method_name == crate::python::constants::INIT_STEM,
                };
            }
            return ExecutionContext::FunctionBody;
        }

        // No functions in scope; check for class body.
        if self
            .scope_stack
            .iter()
            .any(|s| matches!(s, ScopeElement::Class(_)))
        {
            return ExecutionContext::ClassBody;
        }

        self.current_context
    }

    /// Analyze whether an import can be moved to function scope
    fn is_import_movable(&self, import: &DiscoveredImport) -> bool {
        // Check if import has side effects
        if let Some(module_name) = &import.module_name
            && self.is_side_effect_import(module_name)
        {
            return false;
        }

        // Check execution contexts where import is used
        let requires_module_level = import.execution_contexts.iter().any(|ctx| match ctx {
            ExecutionContext::ModuleLevel | ExecutionContext::ClassBody => true,
            ExecutionContext::ClassMethod { is_init } => *is_init,
            ExecutionContext::FunctionBody | ExecutionContext::TypeAnnotation => false,
        });

        !requires_module_level && !import.is_used_in_init
    }

    /// Check if a condition is a `TYPE_CHECKING` check
    fn is_type_checking_condition(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
            Expr::Attribute(attr) => {
                if attr.attr.as_str() == "TYPE_CHECKING"
                    && let Expr::Name(name) = &*attr.value
                {
                    return name.id.as_str() == "typing";
                }
                false
            }
            _ => false,
        }
    }

    /// Check if a module import has side effects
    fn is_side_effect_import(&self, module_name: &str) -> bool {
        matches!(
            module_name,
            "antigravity"
                | "this"
                | "__hello__"
                | "__phello__"
                | "site"
                | "sitecustomize"
                | "usercustomize"
                | "readline"
                | "rlcompleter"
                | "turtle"
                | "tkinter"
                | "webbrowser"
                | "platform"
                | "locale"
                | "os"
                | "sys"
                | "logging"
                | "warnings"
                | "encodings"
                | "pygame"
                | "matplotlib"
        ) || module_name.starts_with('_')
    }

    /// Check if this is a static `importlib.import_module` call
    fn is_static_importlib_call(&self, call: &ExprCall) -> bool {
        match &*call.func {
            // importlib.import_module(...) or il.import_module(...) where il is an alias
            Expr::Attribute(ExprAttribute { attr, value, .. }) => {
                if attr.as_str() == "import_module"
                    && let Expr::Name(ExprName { id, .. }) = &**value
                {
                    let name = id.as_str();
                    // Check if it's importlib directly or an alias to importlib
                    return name == "importlib"
                        || self
                            .lookup_imported_name(name)
                            .is_some_and(|module| module == "importlib");
                }
            }
            // import_module(...) or im(...) where im is an alias
            Expr::Name(ExprName { id, .. }) => {
                let name = id.as_str();
                // Direct check for import_module
                if name == "import_module" && self.has_importlib {
                    return true;
                }
                // Check if this is an alias for import_module
                // Look for imports like "from importlib import import_module as im"
                if let Some(_import_info) = self.imports.iter().find(|imp| {
                    if let ImportType::From = imp.import_type {
                        imp.module_name.as_deref() == Some("importlib")
                            && imp.names.iter().any(|(orig, alias)| {
                                orig == "import_module" && alias.as_deref() == Some(name)
                            })
                    } else {
                        false
                    }
                }) {
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    /// Extract literal module name from `importlib.import_module` call
    fn extract_literal_module_name(&self, call: &ExprCall) -> Option<String> {
        // Only handle static string literals
        if let Some(arg) = call.arguments.args.first()
            && let Expr::StringLiteral(ExprStringLiteral { value, .. }) = arg
        {
            return Some(value.to_str().to_string());
        }
        None
    }

    fn extract_package_context(&self, call: &ExprCall) -> Option<String> {
        // Extract the second argument if it exists (package context for relative imports)
        if call.arguments.args.len() >= 2
            && let Expr::StringLiteral(ExprStringLiteral { value, .. }) = &call.arguments.args[1]
        {
            return Some(value.to_str().to_string());
        }
        None
    }

    /// Record an import statement
    fn record_import(&mut self, stmt: &StmtImport) {
        for alias in &stmt.names {
            let module_name = alias.name.to_string();
            let imported_as = alias
                .asname
                .as_ref()
                .map_or_else(|| module_name.clone(), std::string::ToString::to_string);

            // Track the import mapping
            self.insert_imported_name(imported_as.clone(), module_name.clone());

            // Check if we're importing importlib
            if module_name == "importlib" {
                self.has_importlib = true;
            }

            let import = DiscoveredImport {
                module_name: Some(module_name),
                names: vec![(
                    alias.name.to_string(),
                    alias.asname.as_ref().map(std::string::ToString::to_string),
                )],
                location: self.current_location(),
                range: stmt.range,
                level: 0,
                import_type: ImportType::Direct,
                execution_contexts: FxIndexSet::default(),
                is_used_in_init: false,
                is_movable: false,
                is_type_checking_only: self.in_type_checking,
                package_context: None,
            };
            self.imports.push(import);
        }
    }

    /// Record a from import statement
    fn record_import_from(&mut self, stmt: &StmtImportFrom) {
        let module_name = stmt.module.as_ref().map(std::string::ToString::to_string);

        let names: Vec<(String, Option<String>)> = stmt
            .names
            .iter()
            .map(|alias| {
                let name = alias.name.to_string();
                let asname = alias.asname.as_ref().map(std::string::ToString::to_string);

                // Track import mappings (also for relative imports with no module_name)
                let imported_as = asname.clone().unwrap_or_else(|| name.clone());
                // Encode level into the key to avoid conflating different relatives
                let module_key = match (module_name.as_ref(), stmt.level) {
                    (Some(m), 0) => format!("{m}.{name}"),
                    (Some(m), level) => format!(".{}{}.{}", ".".repeat(level as usize), m, name),
                    (None, level) if level > 0 => {
                        format!(".{}{}", ".".repeat(level as usize), name)
                    }
                    (None, _) => name.clone(),
                };
                self.insert_imported_name(imported_as.clone(), module_key);

                // Check if we're importing import_module from importlib
                if module_name.as_deref() == Some("importlib") && name == "import_module" {
                    self.has_importlib = true;
                }

                (name, asname)
            })
            .collect();

        let import_type = if stmt.level > 0 {
            ImportType::Relative { level: stmt.level }
        } else {
            ImportType::From
        };

        let import = DiscoveredImport {
            module_name,
            names,
            location: self.current_location(),
            range: stmt.range,
            level: stmt.level,
            import_type,
            execution_contexts: FxIndexSet::default(),
            is_used_in_init: false,
            is_movable: false,
            is_type_checking_only: self.in_type_checking,
            package_context: None,
        };
        self.imports.push(import);
    }
}

impl<'a> Visitor<'a> for ImportDiscoveryVisitor<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Import(import_stmt) => {
                log::debug!("Processing import statement");
                self.record_import(import_stmt);
            }
            Stmt::ImportFrom(import_from) => {
                log::debug!("Processing import from statement");
                self.record_import_from(import_from);
            }
            Stmt::Assign(_assign_stmt) => {
                log::debug!("Processing assignment statement");
                // Just walk the statement - ImportlibStatic imports are handled in visit_expr
                walk_stmt(self, stmt);
                return; // Don't call walk_stmt again
            }
            Stmt::FunctionDef(func) => {
                self.scope_stack
                    .push(ScopeElement::Function(func.name.to_string()));
                self.imported_names_stack.push(FxIndexMap::default());
                // Visit the function body
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                self.imported_names_stack.pop();
                return; // Don't call walk_stmt again
            }
            Stmt::ClassDef(class) => {
                self.scope_stack
                    .push(ScopeElement::Class(class.name.to_string()));
                self.imported_names_stack.push(FxIndexMap::default());
                // Visit the class body
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                self.imported_names_stack.pop();
                return;
            }
            Stmt::If(if_stmt) => {
                // Check if this is a TYPE_CHECKING block
                let was_type_checking = self.in_type_checking;
                if self.is_type_checking_condition(&if_stmt.test) {
                    self.in_type_checking = true;
                }

                self.scope_stack.push(ScopeElement::If);
                walk_stmt(self, stmt);
                self.scope_stack.pop();

                // Restore the previous type checking state
                self.in_type_checking = was_type_checking;
                return;
            }
            Stmt::While(_) => {
                self.scope_stack.push(ScopeElement::While);
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            Stmt::For(_) => {
                self.scope_stack.push(ScopeElement::For);
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            Stmt::With(_) => {
                self.scope_stack.push(ScopeElement::With);
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            Stmt::Try(_) => {
                self.scope_stack.push(ScopeElement::Try);
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            _ => {}
        }

        // For other statement types, use default traversal
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Call(call) => {
                // Check for importlib.import_module("literal")
                if self.is_static_importlib_call(call)
                    && let Some(module_name) = self.extract_literal_module_name(call)
                {
                    let package_context = self.extract_package_context(call);
                    log::debug!(
                        "Found static importlib call for module: {module_name}, package: \
                         {package_context:?}"
                    );

                    // Determine import level for relative imports
                    let level = if module_name.starts_with('.') {
                        module_name.chars().take_while(|&c| c == '.').count() as u32
                    } else {
                        0
                    };

                    // Track this as an import
                    let import = DiscoveredImport {
                        module_name: Some(module_name.clone()),
                        names: vec![], // No specific names for direct module import
                        location: self.current_location(),
                        range: call.range,
                        level,
                        import_type: ImportType::ImportlibStatic,
                        execution_contexts: FxIndexSet::default(),
                        is_used_in_init: false,
                        is_movable: false,
                        is_type_checking_only: self.in_type_checking,
                        package_context,
                    };
                    self.imports.push(import);

                    // Do not add to imported_names: importlib.import_module returns a value
                    // assigned to a variable, but it does not bind `module_name` in scope.
                }
            }
            Expr::Name(ExprName { id, range, .. }) => {
                let name = id.to_string();

                // Check if this is an imported name
                if self.lookup_imported_name(&name).is_some() {
                    let context = self.get_current_execution_context();

                    // Record usage
                    self.name_usage
                        .entry(name.clone())
                        .or_default()
                        .push(ImportUsage {
                            _location: *range,
                            _context: context,
                            _name_used: name.clone(),
                        });

                    // Update the import's execution contexts
                    if let Some(module_source) = self.lookup_imported_name(&name).cloned() {
                        // Find the corresponding import and update its contexts
                        for import in &mut self.imports {
                            if import.module_name.as_ref() == Some(&module_source)
                                || import
                                    .names
                                    .iter()
                                    .any(|(n, alias)| alias.as_ref().unwrap_or(n) == &name)
                            {
                                import.execution_contexts.insert(context);
                                if matches!(
                                    context,
                                    ExecutionContext::ClassMethod { is_init: true }
                                ) {
                                    import.is_used_in_init = true;
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        // Continue traversal
        walk_expr(self, expr);
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_ast::visitor::Visitor;
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_module_level_import() {
        let source = r"
import os
from sys import path
";
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].module_name, Some("os".to_string()));
        assert!(matches!(imports[0].location, ImportLocation::Module));
        assert!(matches!(imports[0].import_type, ImportType::Direct));
        assert_eq!(imports[1].module_name, Some("sys".to_string()));
        assert_eq!(imports[1].names, vec![("path".to_string(), None)]);
        assert!(matches!(imports[1].import_type, ImportType::From));
    }

    #[test]
    fn test_function_scoped_import() {
        let source = r"
def my_function():
    import json
    from datetime import datetime
    return json.dumps({})
";
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].module_name, Some("json".to_string()));
        assert!(matches!(
            imports[0].location,
            ImportLocation::Function(ref name) if name == "my_function"
        ));
        assert!(matches!(imports[0].import_type, ImportType::Direct));
        assert_eq!(imports[1].module_name, Some("datetime".to_string()));
        assert_eq!(imports[1].names, vec![("datetime".to_string(), None)]);
        assert!(matches!(imports[1].import_type, ImportType::From));
    }

    #[test]
    fn test_class_method_import() {
        let source = r"
class MyClass:
    def method(self):
        from collections import defaultdict
        return defaultdict(list)
";
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 1);
        assert!(matches!(
            imports[0].location,
            ImportLocation::Method { ref class, ref method } if class == "MyClass" && method == "method"
        ));
    }

    #[test]
    fn test_conditional_import() {
        let source = r#"
if True:
    import platform
    if platform.system() == "Windows":
        import winreg
"#;
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 2);
        assert!(matches!(
            imports[0].location,
            ImportLocation::Conditional { depth: 1 }
        ));
        assert!(matches!(
            imports[1].location,
            ImportLocation::Conditional { depth: 2 }
        ));
    }

    #[test]
    fn test_nested_function_in_method_not_misclassified() {
        let source = r"
class MyClass:
    def method(self):
        def nested_function():
            import os
            return os.path.join('a', 'b')
        return nested_function()
";
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 1);
        // The import should be classified as Nested, not Method
        assert!(matches!(
            imports[0].location,
            ImportLocation::Nested(ref scopes) if scopes.len() == 3 &&
                matches!(&scopes[0], ScopeElement::Class(c) if c == "MyClass") &&
                matches!(&scopes[1], ScopeElement::Function(m) if m == "method") &&
                matches!(&scopes[2], ScopeElement::Function(f) if f == "nested_function")
        ));
    }

    #[test]
    fn test_importlib_static_discovery() {
        let source = r#"
import importlib
from importlib import import_module

# Direct importlib.import_module call
mod1 = importlib.import_module("json")

# Using imported import_module
mod2 = import_module("datetime")

# Dynamic import (should not be discovered)
module_name = "os"
mod3 = importlib.import_module(module_name)

# In a function
def load_module():
    return importlib.import_module("collections")
"#;
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
        let imports = visitor.into_imports();

        // Should have: importlib, import_module, json, datetime, collections
        assert_eq!(imports.len(), 5);

        // First two are regular imports
        assert_eq!(imports[0].module_name, Some("importlib".to_string()));
        assert!(matches!(imports[0].import_type, ImportType::Direct));

        assert_eq!(imports[1].module_name, Some("importlib".to_string()));
        assert_eq!(imports[1].names, vec![("import_module".to_string(), None)]);
        assert!(matches!(imports[1].import_type, ImportType::From));

        // Static importlib calls
        assert_eq!(imports[2].module_name, Some("json".to_string()));
        assert!(matches!(
            imports[2].import_type,
            ImportType::ImportlibStatic
        ));
        assert!(matches!(imports[2].location, ImportLocation::Module));

        assert_eq!(imports[3].module_name, Some("datetime".to_string()));
        assert!(matches!(
            imports[3].import_type,
            ImportType::ImportlibStatic
        ));
        assert!(matches!(imports[3].location, ImportLocation::Module));

        assert_eq!(imports[4].module_name, Some("collections".to_string()));
        assert!(matches!(
            imports[4].import_type,
            ImportType::ImportlibStatic
        ));
        assert!(matches!(
            imports[4].location,
            ImportLocation::Function(ref name) if name == "load_module"
        ));
    }

    #[test]
    fn test_relative_imports() {
        let source = r"
from . import utils
from .. import parent_module
from ...package import sibling
";
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 3);

        assert_eq!(imports[0].level, 1);
        assert!(matches!(
            imports[0].import_type,
            ImportType::Relative { level: 1 }
        ));

        assert_eq!(imports[1].level, 2);
        assert!(matches!(
            imports[1].import_type,
            ImportType::Relative { level: 2 }
        ));

        assert_eq!(imports[2].level, 3);
        assert!(matches!(
            imports[2].import_type,
            ImportType::Relative { level: 3 }
        ));
    }
}
