use ruff_python_ast::{Expr, ExprContext, Stmt};
use ruff_text_size::TextRange;
use rustc_hash::FxHashMap;

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

/// Enhanced import information with semantic analysis
#[derive(Debug, Clone)]
pub struct EnhancedImportInfo {
    /// Original import information
    pub base: ImportInfo,
    /// All usages of this import
    pub usages: Vec<ImportUsage>,
}

impl EnhancedImportInfo {
    /// Check if this import has any runtime usage
    pub fn has_runtime_usage(&self) -> bool {
        self.usages
            .iter()
            .any(|u| u.usage_context.requires_runtime())
    }

    /// Check if all usages are in deferred contexts
    pub fn is_deferred_only(&self) -> bool {
        !self.usages.is_empty() && self.usages.iter().all(|u| u.usage_context.is_deferred())
    }

    /// Get the most restrictive context (runtime > deferred)
    pub fn primary_context(&self) -> Option<ExecutionContext> {
        // If any usage is runtime, that's the primary context
        if self.has_runtime_usage() {
            self.usages
                .iter()
                .find(|u| u.usage_context.requires_runtime())
                .map(|u| u.usage_context)
        } else {
            self.usages.first().map(|u| u.usage_context)
        }
    }
}

/// Visitor that tracks semantic context during AST traversal
pub struct SemanticImportVisitor {
    /// Current execution context
    current_context: ExecutionContext,
    /// Stack of contexts for nested scopes
    context_stack: Vec<ExecutionContext>,
    /// Import name to module mapping (for resolving usage)
    import_to_module: FxHashMap<String, String>,
    /// Module to usage tracking
    module_usages: FxHashMap<String, Vec<ImportUsage>>,
    /// Names that are imported
    imported_names: FxHashSet<String>,
}

impl Default for SemanticImportVisitor {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticImportVisitor {
    pub fn new() -> Self {
        Self {
            current_context: ExecutionContext::ModuleLevel,
            context_stack: Vec::new(),
            import_to_module: FxHashMap::default(),
            module_usages: FxHashMap::default(),
            imported_names: FxHashSet::default(),
        }
    }

    /// Register an import for tracking
    pub fn register_import(&mut self, import_name: &str, module_name: &str) {
        self.import_to_module
            .insert(import_name.to_string(), module_name.to_string());
        self.imported_names.insert(import_name.to_string());
    }

    /// Get all usages for a module
    pub fn get_module_usages(&self, module_name: &str) -> Vec<ImportUsage> {
        self.module_usages
            .get(module_name)
            .cloned()
            .unwrap_or_default()
    }

    fn push_context(&mut self, context: ExecutionContext) {
        self.context_stack.push(self.current_context);
        self.current_context = context;
    }

    fn pop_context(&mut self) {
        if let Some(context) = self.context_stack.pop() {
            self.current_context = context;
        }
    }

    fn track_name_usage(&mut self, name: &str, location: TextRange) {
        // Check if this name is an imported name
        if self.imported_names.contains(name)
            && let Some(module_name) = self.import_to_module.get(name).cloned()
        {
            let usage = ImportUsage {
                name: name.to_string(),
                import_name: name.to_string(),
                usage_context: self.current_context,
                location,
            };
            self.module_usages
                .entry(module_name)
                .or_default()
                .push(usage);
        }
    }

    pub fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::FunctionDef(func) => {
                // Function name is defined at current level
                // But body is deferred
                self.push_context(ExecutionContext::FunctionBody);
                for stmt in &func.body {
                    self.visit_stmt(stmt);
                }
                self.pop_context();
            }
            // Note: ruff_python_ast uses is_async flag in FunctionDef
            // No separate AsyncFunctionDef variant
            Stmt::ClassDef(class) => {
                // Class body executes at definition time
                self.push_context(ExecutionContext::ClassBody);
                for stmt in &class.body {
                    self.visit_stmt(stmt);
                }
                self.pop_context();
            }
            Stmt::If(if_stmt) => {
                // Check for TYPE_CHECKING blocks
                if self.is_type_checking_block(&if_stmt.test) {
                    self.push_context(ExecutionContext::TypeCheckingBlock);
                    for stmt in &if_stmt.body {
                        self.visit_stmt(stmt);
                    }
                    self.pop_context();

                    // Don't visit elif/else in TYPE_CHECKING blocks
                } else {
                    // Normal if statement
                    self.visit_expr(&if_stmt.test);
                    for stmt in &if_stmt.body {
                        self.visit_stmt(stmt);
                    }
                    for elif in &if_stmt.elif_else_clauses {
                        if let Some(condition) = &elif.test {
                            self.visit_expr(condition);
                        }
                        for stmt in &elif.body {
                            self.visit_stmt(stmt);
                        }
                    }
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                // Type annotation context
                self.push_context(ExecutionContext::TypeAnnotation);
                self.visit_expr(&ann_assign.annotation);
                self.pop_context();

                // Value is in current context
                if let Some(value) = &ann_assign.value {
                    self.visit_expr(value);
                }
            }
            // Handle other statements
            _ => {
                self.visit_stmt_generic(stmt);
            }
        }
    }

    pub fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Name(name) if matches!(name.ctx, ExprContext::Load) => {
                self.track_name_usage(&name.id, name.range);
            }
            // Recursively visit other expressions
            _ => {
                self.visit_expr_generic(expr);
            }
        }
    }

    fn is_type_checking_block(&self, expr: &Expr) -> bool {
        // Simple check for TYPE_CHECKING
        // In a real implementation, this would resolve the name properly
        if let Expr::Name(name) = expr {
            name.id == "TYPE_CHECKING"
        } else {
            false
        }
    }

    // Generic visitors for recursion
    fn visit_stmt_generic(&mut self, stmt: &Stmt) {
        // This would use a macro or visitor trait in real implementation
        // For now, just handle common cases
        match stmt {
            Stmt::Expr(expr_stmt) => self.visit_expr(&expr_stmt.value),
            Stmt::Assign(assign) => {
                self.visit_expr(&assign.value);
                for target in &assign.targets {
                    self.visit_expr(target);
                }
            }
            Stmt::Return(ret) => {
                if let Some(value) = &ret.value {
                    self.visit_expr(value);
                }
            }
            // ... handle other statement types
            _ => {}
        }
    }

    fn visit_expr_generic(&mut self, expr: &Expr) {
        // This would use a macro or visitor trait in real implementation
        match expr {
            Expr::Call(call) => {
                self.visit_expr(&call.func);
                for arg in &call.arguments.args {
                    self.visit_expr(arg);
                }
            }
            Expr::Attribute(attr) => {
                self.visit_expr(&attr.value);
            }
            Expr::BinOp(binop) => {
                self.visit_expr(&binop.left);
                self.visit_expr(&binop.right);
            }
            // ... handle other expression types
            _ => {}
        }
    }
}

use rustc_hash::FxHashSet;

/// Analyze imports with semantic context
pub fn analyze_imports_semantic(imports: Vec<ImportInfo>, ast: &[Stmt]) -> Vec<EnhancedImportInfo> {
    let mut visitor = SemanticImportVisitor::new();

    // Register all imports
    for import in &imports {
        let import_name = import
            .module_name
            .split('.')
            .next_back()
            .unwrap_or(&import.module_name);
        visitor.register_import(import_name, &import.module_name);

        // Also register any imported names from 'from' imports
        for (name, alias) in &import.imported_names {
            visitor.register_import(name, &import.module_name);
            if let Some(a) = alias {
                visitor.register_import(a, &import.module_name);
            }
        }
    }

    // Visit the AST to track usage
    for stmt in ast {
        visitor.visit_stmt(stmt);
    }

    // Build enhanced import info
    imports
        .into_iter()
        .map(|import| {
            let usages = visitor.get_module_usages(&import.module_name);
            EnhancedImportInfo {
                base: import,
                usages,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_context() {
        assert!(ExecutionContext::ModuleLevel.requires_runtime());
        assert!(ExecutionContext::ClassBody.requires_runtime());
        assert!(!ExecutionContext::FunctionBody.requires_runtime());
        assert!(!ExecutionContext::TypeAnnotation.requires_runtime());

        assert!(ExecutionContext::FunctionBody.is_deferred());
        assert!(!ExecutionContext::ModuleLevel.is_deferred());
    }

    #[test]
    fn test_enhanced_import_info() {
        let import = ImportInfo {
            module_name: "test_module".to_string(),
            imported_names: vec![],
            import_type: ImportType::ThirdParty,
            line_number: 1,
        };

        let enhanced = EnhancedImportInfo {
            base: import,
            usages: vec![ImportUsage {
                name: "test_func".to_string(),
                import_name: "test_func".to_string(),
                usage_context: ExecutionContext::FunctionBody,
                location: TextRange::default(),
            }],
        };

        assert!(!enhanced.has_runtime_usage());
        assert!(enhanced.is_deferred_only());
    }
}
