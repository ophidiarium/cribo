//! Enhanced unused import analysis and trimming using rustpython-unparser
//!
//! This module builds upon the existing unused import detection to provide
//! actual code transformation capabilities, removing unused imports and
//! generating clean Python code using AST rewriting techniques.

use anyhow::{Context, Result};
use rustpython_parser::ast::{self, Mod, Stmt};
use rustpython_parser::{Mode, parse};
use std::collections::HashSet;
use unparser::Unparser;

use crate::unused_imports_simple::{UnusedImport, UnusedImportAnalyzer};

/// Enhanced unused import trimmer that provides AST-based code transformation
pub struct UnusedImportTrimmer {
    analyzer: UnusedImportAnalyzer,
}

/// Result of trimming unused imports from Python code
#[derive(Debug, Clone)]
pub struct TrimResult {
    /// The transformed Python code with unused imports removed
    pub code: String,
    /// List of unused imports that were removed
    pub removed_imports: Vec<UnusedImport>,
    /// Whether any changes were made to the original code
    pub has_changes: bool,
}

/// Configuration for import trimming behavior
#[derive(Debug, Clone)]
pub struct TrimConfig {
    /// Whether to preserve imports with side effects
    pub preserve_side_effects: bool,
    /// Whether to preserve star imports
    pub preserve_star_imports: bool,
    /// Whether to preserve __future__ imports
    pub preserve_future_imports: bool,
    /// Custom patterns for imports to always preserve
    pub preserve_patterns: Vec<String>,
}

impl Default for TrimConfig {
    fn default() -> Self {
        Self {
            preserve_side_effects: true,
            preserve_star_imports: true,
            preserve_future_imports: true,
            preserve_patterns: vec![],
        }
    }
}

impl UnusedImportTrimmer {
    /// Create a new unused import trimmer
    pub fn new() -> Self {
        Self {
            analyzer: UnusedImportAnalyzer::new(),
        }
    }

    /// Analyze and trim unused imports from Python source code
    ///
    /// This method:
    /// 1. Parses the Python source into an AST
    /// 2. Identifies unused imports using the existing analyzer
    /// 3. Removes unused import statements from the AST
    /// 4. Generates clean Python code using rustpython-unparser
    ///
    /// # Arguments
    /// * `source` - The Python source code to analyze and trim
    /// * `config` - Configuration for trimming behavior
    ///
    /// # Returns
    /// * `Ok(TrimResult)` - The trimmed code and metadata about changes
    /// * `Err` - If parsing or unparsing fails
    pub fn trim_unused_imports(&mut self, source: &str, config: &TrimConfig) -> Result<TrimResult> {
        // Step 1: Analyze for unused imports
        let unused_imports = self
            .analyzer
            .analyze_file(source)
            .context("Failed to analyze unused imports")?;

        if unused_imports.is_empty() {
            return Ok(TrimResult {
                code: source.to_string(),
                removed_imports: vec![],
                has_changes: false,
            });
        }

        // Step 2: Parse source into AST
        let parsed =
            parse(source, Mode::Module, "module").context("Failed to parse Python source code")?;

        let Mod::Module(mut module) = parsed else {
            return Err(anyhow::anyhow!("Expected module, got other AST node type"));
        };

        // Step 3: Filter unused imports based on config
        let imports_to_remove = self.filter_imports_to_remove(&unused_imports, config);

        if imports_to_remove.is_empty() {
            return Ok(TrimResult {
                code: source.to_string(),
                removed_imports: vec![],
                has_changes: false,
            });
        }

        // Step 4: Build set of import names to remove for efficient lookup
        let remove_set: HashSet<String> = imports_to_remove
            .iter()
            .map(|import| import.name.clone())
            .collect();

        // Step 5: Transform AST by removing unused import statements
        let original_count = module.body.len();
        module.body = self.filter_statements(&module.body, &remove_set)?;
        let has_changes = module.body.len() < original_count;

        // Step 6: Generate clean Python code using rustpython-unparser
        let mut unparser = Unparser::new();
        let code = self
            .unparse_module(&mut unparser, &module)
            .context("Failed to generate Python code from AST")?;

        Ok(TrimResult {
            code,
            removed_imports: imports_to_remove,
            has_changes,
        })
    }

    /// Analyze source code without making changes
    ///
    /// Useful for preview/dry-run mode to see what would be changed
    pub fn analyze_only(&mut self, source: &str, config: &TrimConfig) -> Result<Vec<UnusedImport>> {
        let unused_imports = self
            .analyzer
            .analyze_file(source)
            .context("Failed to analyze unused imports")?;

        Ok(self.filter_imports_to_remove(&unused_imports, config))
    }

    /// Filter unused imports based on configuration settings
    fn filter_imports_to_remove(
        &self,
        unused_imports: &[UnusedImport],
        config: &TrimConfig,
    ) -> Vec<UnusedImport> {
        unused_imports
            .iter()
            .filter(|import| self.should_remove_import(import, config))
            .cloned()
            .collect()
    }

    /// Determine if an import should be removed based on config
    fn should_remove_import(&self, import: &UnusedImport, config: &TrimConfig) -> bool {
        // Check if it's a __future__ import
        if config.preserve_future_imports && import.qualified_name.starts_with("__future__") {
            return false;
        }

        // Check custom preserve patterns
        for pattern in &config.preserve_patterns {
            if import.qualified_name.contains(pattern) {
                return false;
            }
        }

        // For now, always remove - the analyzer already handles side effects and star imports
        // In the future, we can add more sophisticated filtering here
        true
    }

    /// Filter AST statements to remove unused import statements
    fn filter_statements(
        &self,
        statements: &[Stmt],
        remove_set: &HashSet<String>,
    ) -> Result<Vec<Stmt>> {
        let mut filtered_statements = Vec::new();

        for stmt in statements {
            match stmt {
                Stmt::Import(import_stmt) => {
                    // Filter individual aliases within import statement
                    let filtered_aliases: Vec<_> = import_stmt
                        .names
                        .iter()
                        .filter(|alias| {
                            let local_name = alias
                                .asname
                                .as_ref()
                                .map(|n| n.as_str())
                                .unwrap_or_else(|| alias.name.as_str());
                            !remove_set.contains(local_name)
                        })
                        .cloned()
                        .collect();

                    // Only keep the import statement if it has remaining aliases
                    if !filtered_aliases.is_empty() {
                        let mut new_import = import_stmt.clone();
                        new_import.names = filtered_aliases;
                        filtered_statements.push(Stmt::Import(new_import));
                    }
                }
                Stmt::ImportFrom(import_from_stmt) => {
                    // Filter individual aliases within from import statement
                    let filtered_aliases: Vec<_> = import_from_stmt
                        .names
                        .iter()
                        .filter(|alias| {
                            let local_name = alias
                                .asname
                                .as_ref()
                                .map(|n| n.as_str())
                                .unwrap_or_else(|| alias.name.as_str());
                            !remove_set.contains(local_name)
                        })
                        .cloned()
                        .collect();

                    // Only keep the import statement if it has remaining aliases
                    if !filtered_aliases.is_empty() {
                        let mut new_import = import_from_stmt.clone();
                        new_import.names = filtered_aliases;
                        filtered_statements.push(Stmt::ImportFrom(new_import));
                    }
                }
                _ => {
                    // Keep all non-import statements as-is
                    filtered_statements.push(stmt.clone());
                }
            }
        }

        Ok(filtered_statements)
    }

    /// Generate Python code from AST using rustpython-unparser
    fn unparse_module(&self, unparser: &mut Unparser, module: &ast::ModModule) -> Result<String> {
        // Use rustpython-unparser to convert AST back to Python code
        for stmt in &module.body {
            unparser.unparse_stmt(stmt);
        }

        Ok(unparser.source.clone())
    }
}

impl Default for UnusedImportTrimmer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::{assert_snapshot, with_settings};

    fn format_trim_result(result: &TrimResult) -> String {
        let mut output = String::new();

        output.push_str(&format!("Has changes: {}\n", result.has_changes));
        output.push_str(&format!(
            "Removed imports count: {}\n",
            result.removed_imports.len()
        ));

        if !result.removed_imports.is_empty() {
            output.push_str("Removed imports:\n");
            // Sort removed imports by name for deterministic output
            let mut sorted_imports = result.removed_imports.clone();
            sorted_imports.sort_by(|a, b| a.name.cmp(&b.name));
            for import in &sorted_imports {
                output.push_str(&format!(
                    "  - {} ({})\n",
                    import.name, import.qualified_name
                ));
            }
        }

        output.push_str("Transformed code:\n");
        output.push_str(&result.code);

        output
    }

    #[test]
    fn test_basic_unused_import_trimming() {
        let mut trimmer = UnusedImportTrimmer::new();
        let config = TrimConfig::default();

        let source = r#"import os
import sys
from pathlib import Path

def main():
    print(sys.version)
    p = Path(".")
    print(p)

if __name__ == "__main__":
    main()
"#;

        let result = trimmer.trim_unused_imports(source, &config).unwrap();

        with_settings!({
            description => "Basic unused import trimming removes only unused imports"
        }, {
            assert_snapshot!(format_trim_result(&result));
        });
    }

    #[test]
    fn test_partial_import_trimming() {
        let mut trimmer = UnusedImportTrimmer::new();
        let config = TrimConfig::default();

        let source = r#"from typing import List, Dict, Optional, Union

def process_data(items: List[str]) -> Dict[str, int]:
    result = {}
    for item in items:
        result[item] = len(item)
    return result
"#;

        let result = trimmer.trim_unused_imports(source, &config).unwrap();

        with_settings!({
            description => "Partial import trimming removes only unused items from from-imports"
        }, {
            assert_snapshot!(format_trim_result(&result));
        });
    }

    #[test]
    fn test_no_unused_imports() {
        let mut trimmer = UnusedImportTrimmer::new();
        let config = TrimConfig::default();

        let source = r#"import math
import json

def calculate(x):
    result = math.sqrt(x)
    data = json.dumps({"result": result})
    return data
"#;

        let result = trimmer.trim_unused_imports(source, &config).unwrap();

        with_settings!({
            description => "Code with no unused imports remains unchanged"
        }, {
            assert_snapshot!(format_trim_result(&result));
        });
    }

    #[test]
    fn test_complex_import_scenarios() {
        let mut trimmer = UnusedImportTrimmer::new();
        let config = TrimConfig::default();

        let source = r#"import os
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
"#;

        let result = trimmer.trim_unused_imports(source, &config).unwrap();

        with_settings!({
            description => "Complex import scenario with mixed used and unused imports"
        }, {
            assert_snapshot!(format_trim_result(&result));
        });
    }

    #[test]
    fn test_future_imports_preserved() {
        let mut trimmer = UnusedImportTrimmer::new();
        let config = TrimConfig::default();

        let source = r#"from __future__ import annotations, print_function
import sys
import json

def main():
    print(sys.version)
"#;

        let result = trimmer.trim_unused_imports(source, &config).unwrap();

        with_settings!({
            description => "Future imports are preserved by default configuration"
        }, {
            assert_snapshot!(format_trim_result(&result));
        });
    }

    #[test]
    fn test_analyze_only_mode() {
        let mut trimmer = UnusedImportTrimmer::new();
        let config = TrimConfig::default();

        let source = r#"import os
import sys
from pathlib import Path

def main():
    print(sys.version)
    p = Path(".")
    print(p)
"#;

        let unused_imports = trimmer.analyze_only(source, &config).unwrap();

        let mut output = String::new();
        output.push_str(&format!("Unused imports count: {}\n", unused_imports.len()));
        for import in &unused_imports {
            output.push_str(&format!(
                "  - {} ({})\n",
                import.name, import.qualified_name
            ));
        }

        with_settings!({
            description => "Analyze-only mode identifies unused imports without modifying code"
        }, {
            assert_snapshot!(output);
        });
    }

    #[test]
    fn test_custom_preserve_patterns() {
        let mut trimmer = UnusedImportTrimmer::new();
        let config = TrimConfig {
            preserve_patterns: vec!["django".to_string(), "pytest".to_string()],
            ..Default::default()
        };

        let source = r#"import os
import django.setup
import pytest_django
import json

def main():
    pass
"#;

        let result = trimmer.trim_unused_imports(source, &config).unwrap();

        with_settings!({
            description => "Custom preserve patterns keep specified imports even if unused"
        }, {
            assert_snapshot!(format_trim_result(&result));
        });
    }

    #[test]
    fn test_empty_import_statements_removed() {
        let mut trimmer = UnusedImportTrimmer::new();
        let config = TrimConfig::default();

        let source = r#"from typing import Optional, Union
from collections import Counter, deque
import json

def process(data: Optional[str]) -> str:
    return data or "default"
"#;

        let result = trimmer.trim_unused_imports(source, &config).unwrap();

        with_settings!({
            description => "Import statements with all unused items are completely removed"
        }, {
            assert_snapshot!(format_trim_result(&result));
        });
    }
}
