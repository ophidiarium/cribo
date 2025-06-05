use anyhow::Result;
use indexmap::{IndexMap, IndexSet};
use ruff_python_ast::{self as ast, Expr, Stmt};
use ruff_python_parser;

/// Simple unused import analyzer focused on core functionality
pub struct UnusedImportAnalyzer {
    /// All imported names in the module
    imported_names: IndexMap<String, ImportInfo>,
    /// Names that have been used
    used_names: IndexSet<String>,
    /// Names exported via __all__
    exported_names: IndexSet<String>,
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
            imported_names: IndexMap::new(),
            used_names: IndexSet::new(),
            exported_names: IndexSet::new(),
        }
    }

    /// Analyze a Python source file for unused imports
    pub fn analyze_file(&mut self, source: &str) -> Result<Vec<UnusedImport>> {
        // Clear state from any previous analysis to ensure independence
        self.imported_names.clear();
        self.used_names.clear();
        self.exported_names.clear();

        let parsed = ruff_python_parser::parse_module(source)?;

        let module = parsed; // ruff_python_parser::parse_module returns ModModule directly
        {
            // First pass: collect all bindings recursively
            for stmt in &module.syntax().body {
                self.collect_imports_recursive(stmt);
            }

            // Second pass: track usage recursively
            for stmt in &module.syntax().body {
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
        assign.targets.iter().any(
            |target| matches!(target, Expr::Name(name_expr) if name_expr.id.as_str() == "__all__"),
        )
    }

    /// Extract names from __all__ assignment value
    fn extract_names_from_all_assignment(&mut self, assign: &ast::StmtAssign) {
        if let Expr::List(list_expr) = assign.value.as_ref() {
            for element in &list_expr.elts {
                self.process_all_list_element(element);
            }
        }
    }

    /// Process a single element in __all__ list
    fn process_all_list_element(&mut self, element: &ast::Expr) {
        if let Expr::StringLiteral(const_expr) = element {
            let s = &const_expr.value;
            self.exported_names.insert(s.to_string());
        }
    }

    /// Extract the full dotted name from an attribute expression
    /// For example, xml.etree.ElementTree.__name__ -> "xml.etree.ElementTree"
    fn extract_full_dotted_name(expr: &ast::Expr) -> Option<String> {
        match expr {
            Expr::Name(name_expr) => Some(name_expr.id.as_str().to_string()),
            Expr::Attribute(attr_expr) => Self::extract_full_dotted_name(&attr_expr.value)
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
            Stmt::ClassDef(class_def) => {
                self.track_usage_in_class_def(class_def);
            }
            Stmt::Return(return_stmt) => {
                self.track_usage_in_return(return_stmt);
            }
            Stmt::Assign(assign) => {
                self.track_usage_in_expression(&assign.value);
            }
            Stmt::AnnAssign(ann_assign) => {
                self.track_usage_in_ann_assign(ann_assign);
            }
            Stmt::AugAssign(aug_assign) => {
                self.track_usage_in_expression(&aug_assign.value);
            }
            Stmt::For(for_stmt) => {
                self.track_usage_in_for_loop(for_stmt);
            }
            Stmt::While(while_stmt) => {
                self.track_usage_in_while_loop(while_stmt);
            }
            Stmt::If(if_stmt) => {
                self.track_usage_in_if_statement(if_stmt);
            }
            Stmt::Expr(expr_stmt) => {
                self.track_usage_in_expression(&expr_stmt.value);
            }
            Stmt::With(with_stmt) => {
                self.track_usage_in_with_statement(with_stmt);
            }
            _ => {
                // For other statement types, we can add more specific handling later
            }
        }
    }

    /// Track usage in class definition statement
    fn track_usage_in_class_def(&mut self, class_def: &ast::StmtClassDef) {
        // Track usage in class body
        for stmt in &class_def.body {
            self.track_usage_in_statement(stmt);
        }
        // Track usage in decorators
        for decorator in &class_def.decorator_list {
            self.track_usage_in_expression(&decorator.expression);
        }
        // Track usage in base classes
        for base in class_def.bases() {
            self.track_usage_in_expression(base);
        }
    }

    /// Track usage in return statement
    fn track_usage_in_return(&mut self, return_stmt: &ast::StmtReturn) {
        if let Some(value) = &return_stmt.value {
            self.track_usage_in_expression(value);
        }
    }

    /// Track usage in annotated assignment
    fn track_usage_in_ann_assign(&mut self, ann_assign: &ast::StmtAnnAssign) {
        // Track usage in the type annotation
        self.track_usage_in_expression(&ann_assign.annotation);
        // Track usage in the value being assigned
        if let Some(value) = &ann_assign.value {
            self.track_usage_in_expression(value);
        }
    }

    /// Track usage in for loop statement
    fn track_usage_in_for_loop(&mut self, for_stmt: &ast::StmtFor) {
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

    /// Track usage in while loop statement
    fn track_usage_in_while_loop(&mut self, while_stmt: &ast::StmtWhile) {
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

    /// Track usage in if statement
    fn track_usage_in_if_statement(&mut self, if_stmt: &ast::StmtIf) {
        // Track usage in test condition
        self.track_usage_in_expression(&if_stmt.test);
        // Track usage in body
        for stmt in &if_stmt.body {
            self.track_usage_in_statement(stmt);
        }
        // Track usage in elif/else clauses
        for clause in &if_stmt.elif_else_clauses {
            if let Some(test) = &clause.test {
                self.track_usage_in_expression(test);
            }
            for stmt in &clause.body {
                self.track_usage_in_statement(stmt);
            }
        }
    }

    /// Track usage in with statement
    fn track_usage_in_with_statement(&mut self, with_stmt: &ast::StmtWith) {
        // Track usage in context expressions and optional variables
        for item in &with_stmt.items {
            self.track_usage_in_expression(&item.context_expr);
            if let Some(optional_vars) = &item.optional_vars {
                self.track_usage_in_expression(optional_vars);
            }
        }
        // Track usage in body
        for stmt in &with_stmt.body {
            self.track_usage_in_statement(stmt);
        }
    }

    /// Track usage of names in an expression
    fn track_usage_in_expression(&mut self, expr: &ast::Expr) {
        match expr {
            Expr::Name(name_expr) => {
                self.track_name_usage(name_expr);
            }
            Expr::Attribute(attr_expr) => {
                self.track_attribute_usage(expr, attr_expr);
            }
            Expr::Call(call_expr) => {
                self.track_call_usage(call_expr);
            }
            Expr::BinOp(binop_expr) => {
                self.track_usage_in_expression(&binop_expr.left);
                self.track_usage_in_expression(&binop_expr.right);
            }
            Expr::UnaryOp(unaryop_expr) => {
                self.track_usage_in_expression(&unaryop_expr.operand);
            }
            Expr::BoolOp(boolop_expr) => {
                self.track_bool_op_usage(boolop_expr);
            }
            Expr::Compare(compare_expr) => {
                self.track_compare_usage(compare_expr);
            }
            Expr::List(list_expr) => {
                self.track_list_usage(list_expr);
            }
            Expr::Tuple(tuple_expr) => {
                self.track_tuple_usage(tuple_expr);
            }
            Expr::Dict(dict_expr) => {
                self.track_dict_usage(dict_expr);
            }
            Expr::Set(set_expr) => {
                self.track_set_usage(set_expr);
            }
            Expr::Subscript(subscript_expr) => {
                self.track_subscript_usage(subscript_expr);
            }
            Expr::FString(f_string) => {
                self.track_fstring_usage(f_string);
            }
            _ => {
                // For other expression types, we can add more specific handling later
            }
        }
    }

    /// Track usage of a name expression
    fn track_name_usage(&mut self, name_expr: &ast::ExprName) {
        let name = name_expr.id.as_str();
        self.used_names.insert(name.to_string());
    }

    /// Track usage of an attribute expression
    fn track_attribute_usage(&mut self, expr: &ast::Expr, attr_expr: &ast::ExprAttribute) {
        self.process_attribute_usage(expr);
        // Continue with recursive processing
        self.track_usage_in_expression(&attr_expr.value);
    }

    /// Track usage in call expression
    fn track_call_usage(&mut self, call_expr: &ast::ExprCall) {
        self.track_usage_in_expression(&call_expr.func);
        for arg in &call_expr.arguments.args {
            self.track_usage_in_expression(arg);
        }
        for keyword in &call_expr.arguments.keywords {
            self.track_usage_in_expression(&keyword.value);
        }
    }

    /// Track usage in boolean operation
    fn track_bool_op_usage(&mut self, boolop_expr: &ast::ExprBoolOp) {
        for value in &boolop_expr.values {
            self.track_usage_in_expression(value);
        }
    }

    /// Track usage in comparison expression
    fn track_compare_usage(&mut self, compare_expr: &ast::ExprCompare) {
        self.track_usage_in_expression(&compare_expr.left);
        for comparator in &compare_expr.comparators {
            self.track_usage_in_expression(comparator);
        }
    }

    /// Track usage in list expression
    fn track_list_usage(&mut self, list_expr: &ast::ExprList) {
        for element in &list_expr.elts {
            self.track_usage_in_expression(element);
        }
    }

    /// Track usage in tuple expression
    fn track_tuple_usage(&mut self, tuple_expr: &ast::ExprTuple) {
        for element in &tuple_expr.elts {
            self.track_usage_in_expression(element);
        }
    }

    /// Track usage in dictionary expression
    fn track_dict_usage(&mut self, dict_expr: &ast::ExprDict) {
        for item in &dict_expr.items {
            // Handle dictionary key (might be None for dict unpacking)
            if let Some(key) = &item.key {
                self.track_usage_in_expression(key);
            }
            // Handle dictionary value
            self.track_usage_in_expression(&item.value);
        }
    }

    /// Track usage in set expression
    fn track_set_usage(&mut self, set_expr: &ast::ExprSet) {
        for element in &set_expr.elts {
            self.track_usage_in_expression(element);
        }
    }

    /// Track usage in subscript expression
    fn track_subscript_usage(&mut self, subscript_expr: &ast::ExprSubscript) {
        self.track_usage_in_expression(&subscript_expr.value);
        self.track_usage_in_expression(&subscript_expr.slice);
    }

    /// Track usage in f-string expression
    fn track_fstring_usage(&mut self, f_string: &ast::ExprFString) {
        for element in f_string.value.elements() {
            match element {
                ast::FStringElement::Expression(expr_element) => {
                    self.track_fstring_expression_element(expr_element);
                }
                ast::FStringElement::Literal(_) => {
                    // Literal elements don't contain expressions to track
                }
            }
        }
    }

    /// Track usage in a single f-string expression element
    fn track_fstring_expression_element(&mut self, expr_element: &ast::FStringExpressionElement) {
        // Track usage in the expression part of interpolated elements
        self.track_usage_in_expression(&expr_element.expression);

        // Track usage in format spec if present
        if let Some(format_spec) = &expr_element.format_spec {
            self.track_fstring_format_spec(format_spec);
        }
    }

    /// Track usage in f-string format specification
    fn track_fstring_format_spec(&mut self, format_spec: &ast::FStringFormatSpec) {
        for format_element in &format_spec.elements {
            if let ast::FStringElement::Expression(format_expr) = format_element {
                self.track_usage_in_expression(&format_expr.expression);
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
            self.track_usage_in_expression(&decorator.expression);
        }
        // Track usage in arguments default values
        for param_with_default in func_def
            .parameters
            .posonlyargs
            .iter()
            .chain(func_def.parameters.args.iter())
            .chain(func_def.parameters.kwonlyargs.iter())
        {
            if let Some(default) = &param_with_default.default {
                self.track_usage_in_expression(default);
            }
        }
        // Track usage in argument type annotations
        self.process_function_arg_annotations(&func_def.parameters);
        // Track usage in return type annotation
        if let Some(returns) = &func_def.returns {
            self.track_usage_in_expression(returns);
        }
    }

    /// Process function argument annotations
    fn process_function_arg_annotations(&mut self, params: &ast::Parameters) {
        // Process all non-variadic parameters
        for param_with_default in params
            .posonlyargs
            .iter()
            .chain(params.args.iter())
            .chain(params.kwonlyargs.iter())
        {
            if let Some(annotation) = &param_with_default.parameter.annotation {
                self.track_usage_in_expression(annotation);
            }
        }

        // Process variadic parameters
        if let Some(vararg) = &params.vararg {
            if let Some(annotation) = &vararg.annotation {
                self.track_usage_in_expression(annotation);
            }
        }
        if let Some(kwarg) = &params.kwarg {
            if let Some(annotation) = &kwarg.annotation {
                self.track_usage_in_expression(annotation);
            }
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
                self.collect_from_function_body(&func_def.body);
            }
            Stmt::ClassDef(class_def) => {
                self.collect_from_statement_list(&class_def.body);
            }
            Stmt::For(for_stmt) => {
                self.collect_from_for_statement(for_stmt);
            }
            Stmt::If(if_stmt) => {
                self.collect_from_if_statement(if_stmt);
            }
            Stmt::While(while_stmt) => {
                self.collect_from_while_statement(while_stmt);
            }
            Stmt::With(with_stmt) => {
                self.collect_from_statement_list(&with_stmt.body);
            }
            _ => {}
        }
    }

    /// Collect imports from function body
    fn collect_from_function_body(&mut self, body: &[Stmt]) {
        for nested in body {
            self.collect_imports_recursive(nested);
        }
    }

    /// Collect imports from a list of statements
    fn collect_from_statement_list(&mut self, statements: &[Stmt]) {
        for nested in statements {
            self.collect_imports_recursive(nested);
        }
    }

    /// Collect imports from for statement
    fn collect_from_for_statement(&mut self, for_stmt: &ast::StmtFor) {
        self.collect_from_statement_list(&for_stmt.body);
        self.collect_from_statement_list(&for_stmt.orelse);
    }

    /// Collect imports from if statement
    fn collect_from_if_statement(&mut self, if_stmt: &ast::StmtIf) {
        self.collect_from_statement_list(&if_stmt.body);
        for clause in &if_stmt.elif_else_clauses {
            self.collect_from_statement_list(&clause.body);
        }
    }

    /// Collect imports from while statement
    fn collect_from_while_statement(&mut self, while_stmt: &ast::StmtWhile) {
        self.collect_from_statement_list(&while_stmt.body);
        self.collect_from_statement_list(&while_stmt.orelse);
    }

    /// Recursively track usage in nested statements
    fn track_usage_recursive(&mut self, stmt: &Stmt) {
        self.track_usage_in_statement(stmt);

        match stmt {
            Stmt::FunctionDef(func_def) => {
                self.track_usage_in_function_body(&func_def.body);
            }
            Stmt::ClassDef(class_def) => {
                self.track_usage_in_statement_list(&class_def.body);
            }
            Stmt::For(for_stmt) => {
                self.track_usage_in_for_statement_recursive(for_stmt);
            }
            Stmt::If(if_stmt) => {
                self.track_usage_in_if_statement_recursive(if_stmt);
            }
            Stmt::While(while_stmt) => {
                self.track_usage_in_while_statement_recursive(while_stmt);
            }
            Stmt::With(with_stmt) => {
                self.track_usage_in_statement_list(&with_stmt.body);
            }
            _ => {}
        }
    }

    /// Track usage recursively in function body
    fn track_usage_in_function_body(&mut self, body: &[Stmt]) {
        for nested in body {
            self.track_usage_recursive(nested);
        }
    }

    /// Track usage recursively in a list of statements
    fn track_usage_in_statement_list(&mut self, statements: &[Stmt]) {
        for nested in statements {
            self.track_usage_recursive(nested);
        }
    }

    /// Track usage recursively in for statement
    fn track_usage_in_for_statement_recursive(&mut self, for_stmt: &ast::StmtFor) {
        self.track_usage_in_statement_list(&for_stmt.body);
        self.track_usage_in_statement_list(&for_stmt.orelse);
    }

    /// Track usage recursively in if statement
    fn track_usage_in_if_statement_recursive(&mut self, if_stmt: &ast::StmtIf) {
        self.track_usage_in_statement_list(&if_stmt.body);
        for clause in &if_stmt.elif_else_clauses {
            self.track_usage_in_statement_list(&clause.body);
        }
    }

    /// Track usage recursively in while statement
    fn track_usage_in_while_statement_recursive(&mut self, while_stmt: &ast::StmtWhile) {
        self.track_usage_in_statement_list(&while_stmt.body);
        self.track_usage_in_statement_list(&while_stmt.orelse);
    }

    /// Debug method to access imported names
    pub fn get_imported_names(&self) -> &IndexMap<String, ImportInfo> {
        &self.imported_names
    }

    /// Debug method to access used names
    pub fn get_used_names(&self) -> &IndexSet<String> {
        &self.used_names
    }

    /// Debug method to access exported names
    pub fn get_exported_names(&self) -> &IndexSet<String> {
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
