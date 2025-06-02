use anyhow::Result;
use rustpython_parser::ast::{self, Mod, Stmt};
use rustpython_parser::{Mode, parse};
use std::collections::{HashMap, HashSet};

/// Simple unused import analyzer focused on core functionality
pub struct UnusedImportAnalyzer {
    /// All imported names in the module
    imported_names: HashMap<String, ImportInfo>,
    /// Names that have been used
    used_names: HashSet<String>,
    /// Names exported via __all__
    exported_names: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct ImportInfo {
    pub name: String,
    pub qualified_name: String,
    pub is_star_import: bool,
    pub is_side_effect: bool,
}

/// Represents an unused import that was detected
#[derive(Debug, Clone)]
pub struct UnusedImport {
    pub name: String,
    pub qualified_name: String,
}

impl UnusedImportAnalyzer {
    pub fn new() -> Self {
        Self {
            imported_names: HashMap::new(),
            used_names: HashSet::new(),
            exported_names: HashSet::new(),
        }
    }

    /// Analyze a Python source file for unused imports
    pub fn analyze_file(&mut self, source: &str) -> Result<Vec<UnusedImport>> {
        // Clear state from any previous analysis to ensure independence
        self.imported_names.clear();
        self.used_names.clear();
        self.exported_names.clear();

        let parsed = parse(source, Mode::Module, "module")?;

        if let Mod::Module(module) = parsed {
            // First pass: collect all bindings recursively
            for stmt in &module.body {
                self.collect_imports_recursive(stmt);
            }

            // Second pass: track usage recursively
            for stmt in &module.body {
                self.track_usage_recursive(stmt);
            }
        }

        // Find unused imports
        let mut unused_imports = Vec::new();
        for (name, import_info) in &self.imported_names {
            if !self.used_names.contains(name)
                && !self.exported_names.contains(name)
                && !import_info.is_star_import
                && !import_info.is_side_effect
            {
                unused_imports.push(UnusedImport {
                    name: name.clone(),
                    qualified_name: import_info.qualified_name.clone(),
                });
            }
        }

        Ok(unused_imports)
    }

    /// Collect imports from a statement
    fn collect_imports(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Import(import_stmt) => {
                for alias in &import_stmt.names {
                    let module_name = alias.name.as_str();
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map(|n| n.as_str())
                        .unwrap_or(module_name);

                    let is_side_effect = self.is_side_effect_import(module_name);

                    self.imported_names.insert(
                        local_name.to_string(),
                        ImportInfo {
                            name: local_name.to_string(),
                            qualified_name: module_name.to_string(),
                            is_star_import: false,
                            is_side_effect,
                        },
                    );
                }
            }
            Stmt::ImportFrom(import_from_stmt) => {
                let module_name = import_from_stmt
                    .module
                    .as_ref()
                    .map(|m| m.as_str())
                    .unwrap_or("");

                // Skip __future__ imports
                if module_name == "__future__" {
                    return;
                }

                // Check if this is a star import
                if import_from_stmt.names.len() == 1
                    && import_from_stmt.names[0].name.as_str() == "*"
                {
                    self.imported_names.insert(
                        "*".to_string(),
                        ImportInfo {
                            name: "*".to_string(),
                            qualified_name: module_name.to_string(),
                            is_star_import: true,
                            is_side_effect: true,
                        },
                    );
                    return;
                }

                for alias in &import_from_stmt.names {
                    self.process_import_from_alias(alias, module_name);
                }
            }
            _ => {}
        }
    }

    /// Process a single alias from an import_from statement
    fn process_import_from_alias(&mut self, alias: &ast::Alias, module_name: &str) {
        let imported_name = alias.name.as_str();
        let local_name = alias
            .asname
            .as_ref()
            .map(|n| n.as_str())
            .unwrap_or(imported_name);

        let qualified_name = if module_name.is_empty() {
            imported_name.to_string()
        } else {
            format!("{}.{}", module_name, imported_name)
        };

        let is_side_effect = self.is_side_effect_import(&qualified_name);

        self.imported_names.insert(
            local_name.to_string(),
            ImportInfo {
                name: local_name.to_string(),
                qualified_name,
                is_star_import: false,
                is_side_effect,
            },
        );
    }

    /// Collect names exported via __all__
    fn collect_exports(&mut self, stmt: &Stmt) {
        if let Stmt::Assign(assign) = stmt {
            self.process_all_assignment(assign);
        }
    }

    /// Process __all__ assignment to extract exported names
    fn process_all_assignment(&mut self, assign: &ast::StmtAssign) {
        if !self.is_all_assignment(assign) {
            return;
        }
        self.extract_names_from_all_assignment(assign);
    }

    /// Check if this assignment targets __all__
    fn is_all_assignment(&self, assign: &ast::StmtAssign) -> bool {
        assign.targets.iter().any(|target| {
            matches!(target, ast::Expr::Name(name_expr) if name_expr.id.as_str() == "__all__")
        })
    }

    /// Extract names from __all__ assignment value
    fn extract_names_from_all_assignment(&mut self, assign: &ast::StmtAssign) {
        if let ast::Expr::List(list_expr) = assign.value.as_ref() {
            for element in &list_expr.elts {
                self.process_all_list_element(element);
            }
        }
    }

    /// Process a single element in __all__ list
    fn process_all_list_element(&mut self, element: &ast::Expr) {
        if let ast::Expr::Constant(const_expr) = element {
            if let ast::Constant::Str(s) = &const_expr.value {
                self.exported_names.insert(s.to_string());
            }
        }
    }

    /// Extract the full dotted name from an attribute expression
    /// For example, xml.etree.ElementTree.__name__ -> "xml.etree.ElementTree"
    fn extract_full_dotted_name(expr: &ast::Expr) -> Option<String> {
        match expr {
            ast::Expr::Name(name_expr) => Some(name_expr.id.as_str().to_string()),
            ast::Expr::Attribute(attr_expr) => Self::extract_full_dotted_name(&attr_expr.value)
                .map(|base_name| format!("{}.{}", base_name, attr_expr.attr.as_str())),
            _ => None,
        }
    }

    /// Process attribute usage to reduce nesting in track_usage_in_expression
    fn process_attribute_usage(&mut self, expr: &ast::Expr) {
        if let Some(full_name) = Self::extract_full_dotted_name(expr) {
            if self.imported_names.contains_key(&full_name) {
                self.used_names.insert(full_name);
            }
        }
    }

    /// Track usage of names in a statement
    fn track_usage_in_statement(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Import(_) | Stmt::ImportFrom(_) => {
                // Skip import statements themselves
            }
            Stmt::FunctionDef(func_def) => {
                self.process_function_def(func_def);
            }
            Stmt::AsyncFunctionDef(async_func_def) => {
                self.process_async_function_def(async_func_def);
            }
            Stmt::ClassDef(class_def) => {
                // Track usage in class body
                for stmt in &class_def.body {
                    self.track_usage_in_statement(stmt);
                }
                // Track usage in decorators
                for decorator in &class_def.decorator_list {
                    self.track_usage_in_expression(decorator);
                }
                // Track usage in base classes
                for base in &class_def.bases {
                    self.track_usage_in_expression(base);
                }
            }
            Stmt::Return(return_stmt) => {
                if let Some(value) = &return_stmt.value {
                    self.track_usage_in_expression(value);
                }
            }
            Stmt::Assign(assign) => {
                // Track usage in the value being assigned
                self.track_usage_in_expression(&assign.value);
            }
            Stmt::AnnAssign(ann_assign) => {
                // Track usage in the type annotation
                self.track_usage_in_expression(&ann_assign.annotation);
                // Track usage in the value being assigned
                if let Some(value) = &ann_assign.value {
                    self.track_usage_in_expression(value);
                }
            }
            Stmt::AugAssign(aug_assign) => {
                // Track usage in the value being assigned
                self.track_usage_in_expression(&aug_assign.value);
            }
            Stmt::For(for_stmt) => {
                // Track usage in iterator
                self.track_usage_in_expression(&for_stmt.iter);
                // Track usage in body
                for stmt in &for_stmt.body {
                    self.track_usage_in_statement(stmt);
                }
                // Track usage in orelse
                for stmt in &for_stmt.orelse {
                    self.track_usage_in_statement(stmt);
                }
            }
            Stmt::AsyncFor(async_for_stmt) => {
                // Track usage in iterator
                self.track_usage_in_expression(&async_for_stmt.iter);
                // Track usage in body
                for stmt in &async_for_stmt.body {
                    self.track_usage_in_statement(stmt);
                }
                // Track usage in orelse
                for stmt in &async_for_stmt.orelse {
                    self.track_usage_in_statement(stmt);
                }
            }
            Stmt::While(while_stmt) => {
                // Track usage in test condition
                self.track_usage_in_expression(&while_stmt.test);
                // Track usage in body
                for stmt in &while_stmt.body {
                    self.track_usage_in_statement(stmt);
                }
                // Track usage in orelse
                for stmt in &while_stmt.orelse {
                    self.track_usage_in_statement(stmt);
                }
            }
            Stmt::If(if_stmt) => {
                // Track usage in test condition
                self.track_usage_in_expression(&if_stmt.test);
                // Track usage in body
                for stmt in &if_stmt.body {
                    self.track_usage_in_statement(stmt);
                }
                // Track usage in orelse
                for stmt in &if_stmt.orelse {
                    self.track_usage_in_statement(stmt);
                }
            }
            Stmt::Expr(expr_stmt) => {
                self.track_usage_in_expression(&expr_stmt.value);
            }
            _ => {
                // For other statement types, we can add more specific handling later
            }
        }
    }

    /// Track usage of names in an expression
    fn track_usage_in_expression(&mut self, expr: &ast::Expr) {
        match expr {
            ast::Expr::Name(name_expr) => {
                let name = name_expr.id.as_str();
                self.used_names.insert(name.to_string());
            }
            ast::Expr::Attribute(attr_expr) => {
                self.process_attribute_usage(expr);
                // Continue with recursive processing
                self.track_usage_in_expression(&attr_expr.value);
            }
            ast::Expr::Call(call_expr) => {
                self.track_usage_in_expression(&call_expr.func);
                for arg in &call_expr.args {
                    self.track_usage_in_expression(arg);
                }
                for keyword in &call_expr.keywords {
                    self.track_usage_in_expression(&keyword.value);
                }
            }
            ast::Expr::BinOp(binop_expr) => {
                self.track_usage_in_expression(&binop_expr.left);
                self.track_usage_in_expression(&binop_expr.right);
            }
            ast::Expr::UnaryOp(unaryop_expr) => {
                self.track_usage_in_expression(&unaryop_expr.operand);
            }
            ast::Expr::BoolOp(boolop_expr) => {
                for value in &boolop_expr.values {
                    self.track_usage_in_expression(value);
                }
            }
            ast::Expr::Compare(compare_expr) => {
                self.track_usage_in_expression(&compare_expr.left);
                for comparator in &compare_expr.comparators {
                    self.track_usage_in_expression(comparator);
                }
            }
            ast::Expr::List(list_expr) => {
                for element in &list_expr.elts {
                    self.track_usage_in_expression(element);
                }
            }
            ast::Expr::Tuple(tuple_expr) => {
                for element in &tuple_expr.elts {
                    self.track_usage_in_expression(element);
                }
            }
            ast::Expr::Dict(dict_expr) => {
                // Handle dictionary keys (some might be None for dict unpacking)
                dict_expr
                    .keys
                    .iter()
                    .filter_map(|key| key.as_ref())
                    .for_each(|key| self.track_usage_in_expression(key));

                // Handle dictionary values
                for value in &dict_expr.values {
                    self.track_usage_in_expression(value);
                }
            }
            ast::Expr::Set(set_expr) => {
                for element in &set_expr.elts {
                    self.track_usage_in_expression(element);
                }
            }
            ast::Expr::Subscript(subscript_expr) => {
                self.track_usage_in_expression(&subscript_expr.value);
                self.track_usage_in_expression(&subscript_expr.slice);
            }
            ast::Expr::JoinedStr(joined_str) => {
                // Handle f-strings by tracking usage in the values
                for value in &joined_str.values {
                    self.track_usage_in_expression(value);
                }
            }
            ast::Expr::FormattedValue(formatted_value) => {
                // Handle formatted values inside f-strings
                self.track_usage_in_expression(&formatted_value.value);
                if let Some(format_spec) = &formatted_value.format_spec {
                    self.track_usage_in_expression(format_spec);
                }
            }
            _ => {
                // For other expression types, we can add more specific handling later
            }
        }
    }

    /// Process function definition statement to track usage
    fn process_function_def(&mut self, func_def: &ast::StmtFunctionDef) {
        // Track usage in function body
        for stmt in &func_def.body {
            self.track_usage_in_statement(stmt);
        }
        // Track usage in decorators
        for decorator in &func_def.decorator_list {
            self.track_usage_in_expression(decorator);
        }
        // Track usage in arguments default values
        for default in func_def.args.defaults() {
            self.track_usage_in_expression(default);
        }
        // Track usage in argument type annotations
        self.process_function_arg_annotations(&func_def.args);
        // Track usage in return type annotation
        if let Some(returns) = &func_def.returns {
            self.track_usage_in_expression(returns);
        }
    }

    /// Process function argument annotations
    fn process_function_arg_annotations(&mut self, args: &ast::Arguments) {
        for arg in &args.args {
            if let Some(annotation) = &arg.def.annotation {
                self.track_usage_in_expression(annotation);
            }
        }
    }

    /// Process async function definition statement to track usage
    fn process_async_function_def(&mut self, async_func_def: &ast::StmtAsyncFunctionDef) {
        // Track usage in function body
        for stmt in &async_func_def.body {
            self.track_usage_in_statement(stmt);
        }
        // Track usage in decorators
        for decorator in &async_func_def.decorator_list {
            self.track_usage_in_expression(decorator);
        }
        // Track usage in arguments default values
        for default in async_func_def.args.defaults() {
            self.track_usage_in_expression(default);
        }
    }

    /// Check if an import might be a side-effect import
    fn is_side_effect_import(&self, module_name: &str) -> bool {
        // Common patterns for side-effect imports
        // These are imports that are typically used for their side effects
        // rather than for accessing specific names
        // Be conservative - only mark as side-effect if really likely
        let side_effect_patterns = [
            "logging.config",
            "warnings.filterwarnings",
            "multiprocessing.set_start_method",
            "matplotlib.use",
            "django.setup",
            "pytest_django.plugin",
        ];

        side_effect_patterns
            .iter()
            .any(|&pattern| module_name.starts_with(pattern))
    }

    /// Recursively collect imports and exports in nested statements
    fn collect_imports_recursive(&mut self, stmt: &Stmt) {
        self.collect_imports(stmt);
        self.collect_exports(stmt);
        match stmt {
            Stmt::FunctionDef(func_def) => {
                for nested in &func_def.body {
                    self.collect_imports_recursive(nested);
                }
            }
            Stmt::AsyncFunctionDef(async_def) => {
                for nested in &async_def.body {
                    self.collect_imports_recursive(nested);
                }
            }
            Stmt::ClassDef(class_def) => {
                for nested in &class_def.body {
                    self.collect_imports_recursive(nested);
                }
            }
            Stmt::For(for_stmt) => {
                for nested in &for_stmt.body {
                    self.collect_imports_recursive(nested);
                }
                for nested in &for_stmt.orelse {
                    self.collect_imports_recursive(nested);
                }
            }
            Stmt::AsyncFor(async_for_stmt) => {
                for nested in &async_for_stmt.body {
                    self.collect_imports_recursive(nested);
                }
                for nested in &async_for_stmt.orelse {
                    self.collect_imports_recursive(nested);
                }
            }
            Stmt::If(if_stmt) => {
                for nested in &if_stmt.body {
                    self.collect_imports_recursive(nested);
                }
                for nested in &if_stmt.orelse {
                    self.collect_imports_recursive(nested);
                }
            }
            Stmt::While(while_stmt) => {
                for nested in &while_stmt.body {
                    self.collect_imports_recursive(nested);
                }
                for nested in &while_stmt.orelse {
                    self.collect_imports_recursive(nested);
                }
            }
            _ => {}
        }
    }

    /// Recursively track usage in nested statements
    fn track_usage_recursive(&mut self, stmt: &Stmt) {
        self.track_usage_in_statement(stmt);
        match stmt {
            Stmt::FunctionDef(func_def) => {
                for nested in &func_def.body {
                    self.track_usage_recursive(nested);
                }
            }
            Stmt::AsyncFunctionDef(async_def) => {
                for nested in &async_def.body {
                    self.track_usage_recursive(nested);
                }
            }
            Stmt::ClassDef(class_def) => {
                for nested in &class_def.body {
                    self.track_usage_recursive(nested);
                }
            }
            Stmt::For(for_stmt) => {
                for nested in &for_stmt.body {
                    self.track_usage_recursive(nested);
                }
                for nested in &for_stmt.orelse {
                    self.track_usage_recursive(nested);
                }
            }
            Stmt::AsyncFor(async_for_stmt) => {
                for nested in &async_for_stmt.body {
                    self.track_usage_recursive(nested);
                }
                for nested in &async_for_stmt.orelse {
                    self.track_usage_recursive(nested);
                }
            }
            Stmt::If(if_stmt) => {
                for nested in &if_stmt.body {
                    self.track_usage_recursive(nested);
                }
                for nested in &if_stmt.orelse {
                    self.track_usage_recursive(nested);
                }
            }
            Stmt::While(while_stmt) => {
                for nested in &while_stmt.body {
                    self.track_usage_recursive(nested);
                }
                for nested in &while_stmt.orelse {
                    self.track_usage_recursive(nested);
                }
            }
            _ => {}
        }
    }

    /// Debug method to access imported names
    pub fn get_imported_names(&self) -> &HashMap<String, ImportInfo> {
        &self.imported_names
    }

    /// Debug method to access used names
    pub fn get_used_names(&self) -> &HashSet<String> {
        &self.used_names
    }

    /// Debug method to access exported names
    pub fn get_exported_names(&self) -> &HashSet<String> {
        &self.exported_names
    }
}

impl Default for UnusedImportAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::{assert_snapshot, with_settings};

    fn format_unused_imports(unused_imports: &[UnusedImport]) -> String {
        if unused_imports.is_empty() {
            "No unused imports".to_string()
        } else {
            let mut formatted: Vec<_> = unused_imports
                .iter()
                .map(|import| (import.name.clone(), import.qualified_name.clone()))
                .collect();
            formatted.sort();
            formatted
                .into_iter()
                .map(|(name, qualified_name)| format!("- {} ({})", name, qualified_name))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    #[test]
    fn test_unused_import_detection_snapshots() {
        let test_cases = vec![
            (
                "basic_unused_detection",
                r#"
import os
import sys
from pathlib import Path

def main():
    print(sys.version)
    p = Path(".")
    print(p)

if __name__ == "__main__":
    main()
"#,
            ),
            (
                "star_import_handling",
                r#"
from pathlib import *

def main():
    p = Path(".")
    print(p)
"#,
            ),
            (
                "all_export_handling",
                r#"
import os
import json
import sys

__all__ = ["os"]

def main():
    print(sys.version)
"#,
            ),
            (
                "complex_import_scenarios",
                r#"
import os
import sys
import json
from typing import List, Dict, Optional
from collections import defaultdict, Counter
import re

def main():
    # Use sys
    print(sys.version)

    # Use List from typing
    numbers: List[int] = [1, 2, 3]

    # Use defaultdict
    dd = defaultdict(int)
    dd["test"] = 5

    print(f"Numbers: {numbers}")
    print(f"Defaultdict: {dict(dd)}")
"#,
            ),
            (
                "future_imports",
                r#"
from __future__ import annotations, print_function
import sys
import json

def main():
    print(sys.version)
"#,
            ),
            (
                "no_unused_imports",
                r#"
import math
import json

def calculate(x):
    result = math.sqrt(x)
    data = json.dumps({"result": result})
    return data
"#,
            ),
        ];

        let mut output = String::new();

        for (description, source) in test_cases {
            let mut analyzer = UnusedImportAnalyzer::new();
            let unused_imports = analyzer.analyze_file(source).unwrap();

            output.push_str(&format!("## {}\n", description));
            output.push_str(&format!("Source:\n{}\n", source.trim()));
            output.push_str(&format!(
                "Unused imports:\n{}\n\n",
                format_unused_imports(&unused_imports)
            ));
        }

        with_settings!({
            description => "Unused import detection handles various Python import patterns correctly"
        }, {
            assert_snapshot!(output);
        });
    }

    #[test]
    fn test_analyzer_independence_snapshots() {
        let mut analyzer = UnusedImportAnalyzer::new();

        let test_files = vec![
            (
                "file_1_os_unused",
                r#"
import os
import sys

def main():
    print(sys.version)
"#,
            ),
            (
                "file_2_json_unused",
                r#"
import json
import pathlib

def process():
    p = pathlib.Path(".")
    return p
"#,
            ),
            (
                "file_3_all_used",
                r#"
import math

def calculate(x):
    return math.sqrt(x)
"#,
            ),
        ];

        let mut output = String::new();

        for (description, source) in test_files {
            let unused_imports = analyzer.analyze_file(source).unwrap();

            output.push_str(&format!("## {}\n", description));
            output.push_str(&format!("Source:\n{}\n", source.trim()));
            output.push_str(&format!(
                "Unused imports:\n{}\n\n",
                format_unused_imports(&unused_imports)
            ));
        }

        with_settings!({
            description => "Analyzer maintains independence between multiple file analyses"
        }, {
            assert_snapshot!(output);
        });
    }

    // Legacy tests - keeping these for backwards compatibility during transition
    #[test]
    fn test_unused_import_detection() {
        let source = r#"
import os
import sys
from pathlib import Path

def main():
    print(sys.version)
    p = Path(".")
    print(p)

if __name__ == "__main__":
    main()
"#;

        let mut analyzer = UnusedImportAnalyzer::new();
        let unused_imports = analyzer.analyze_file(source).unwrap();

        assert_eq!(unused_imports.len(), 1);
        assert_eq!(unused_imports[0].name, "os");
    }

    #[test]
    fn test_star_import_not_flagged() {
        let source = r#"
from pathlib import *

def main():
    p = Path(".")
    print(p)
"#;

        let mut analyzer = UnusedImportAnalyzer::new();
        let unused_imports = analyzer.analyze_file(source).unwrap();

        // Star imports should not be flagged as unused
        assert_eq!(unused_imports.len(), 0);
    }

    #[test]
    fn test_all_export_prevents_unused_flag() {
        let source = r#"
import os
import json
import sys

__all__ = ["os"]

def main():
    print(sys.version)
"#;

        let mut analyzer = UnusedImportAnalyzer::new();
        let unused_imports = analyzer.analyze_file(source).unwrap();

        // Only json should be flagged as unused:
        // - os is exported via __all__ (so not flagged even though not used)
        // - sys is actually used in the code
        // - json is neither exported nor used
        assert_eq!(unused_imports.len(), 1);
        assert_eq!(unused_imports[0].name, "json");
    }

    #[test]
    fn test_multiple_file_analysis_independence() {
        let mut analyzer = UnusedImportAnalyzer::new();

        // First file analysis - import os but don't use it
        let source1 = r#"
import os
import sys

def main():
    print(sys.version)
"#;

        let unused_imports1 = analyzer.analyze_file(source1).unwrap();
        assert_eq!(unused_imports1.len(), 1);
        assert_eq!(unused_imports1[0].name, "os");

        // Second file analysis - import json but don't use it
        // The previous state should not affect this analysis
        let source2 = r#"
import json
import pathlib

def process():
    p = pathlib.Path(".")
    return p
"#;

        let unused_imports2 = analyzer.analyze_file(source2).unwrap();
        assert_eq!(unused_imports2.len(), 1);
        assert_eq!(unused_imports2[0].name, "json");

        // Third file analysis - no unused imports
        let source3 = r#"
import math

def calculate(x):
    return math.sqrt(x)
"#;

        let unused_imports3 = analyzer.analyze_file(source3).unwrap();
        assert_eq!(unused_imports3.len(), 0);
    }
}
