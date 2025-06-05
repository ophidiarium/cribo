use serpen::ast_rewriter::AstRewriter;

/// Test regular import statements with aliases (non-"from" imports)
/// This specifically tests the code path in ast_rewriter.rs around lines 216-228
#[test]
fn test_regular_import_aliases_collection() {
    let python_code = r#"
# Regular imports with aliases - should be processed
import os as operating_system
import json as j
import sys as system_module
import collections.abc as abc_collections
import urllib.parse as url_parser
import xml.etree.ElementTree as xml_tree
import utils.helpers as helper_utils
import utils.config as config_module

# Regular imports without aliases - should NOT be processed
import math
import random
import datetime

def main():
    pass
"#;

    // Parse the Python code
    let parsed =
        ruff_python_parser::parse_module(python_code).expect("Failed to parse Python code");
    let module_ast = parsed.syntax();

    // Create AST rewriter and collect import aliases
    let mut rewriter = AstRewriter::new(10); // Python 3.10
    rewriter.collect_import_aliases(module_ast, "test_module");

    // Verify that regular import aliases were collected correctly
    let import_aliases = rewriter.import_aliases();

    // Should have collected 8 aliased imports
    assert_eq!(
        import_aliases.len(),
        8,
        "Should collect 8 regular import aliases"
    );

    // Test each expected alias
    let expected_aliases = vec![
        ("operating_system", "os", "os", false, true),
        ("j", "json", "json", false, true),
        ("system_module", "sys", "sys", false, true),
        (
            "abc_collections",
            "collections.abc",
            "collections.abc",
            false,
            true,
        ),
        ("url_parser", "urllib.parse", "urllib.parse", false, true),
        (
            "xml_tree",
            "xml.etree.ElementTree",
            "xml.etree.ElementTree",
            false,
            true,
        ),
        (
            "helper_utils",
            "utils.helpers",
            "utils.helpers",
            false,
            true,
        ),
        ("config_module", "utils.config", "utils.config", false, true),
    ];

    for (alias_name, original_name, module_name, is_from_import, has_explicit_alias) in
        expected_aliases
    {
        assert!(
            import_aliases.contains_key(alias_name),
            "Should contain alias '{}'",
            alias_name
        );

        let import_alias = &import_aliases[alias_name];

        // Verify all properties are set correctly for regular imports
        assert_eq!(
            import_alias.alias_name, alias_name,
            "Alias name should be '{}' for '{}'",
            alias_name, alias_name
        );
        assert_eq!(
            import_alias.original_name, original_name,
            "Original name should be '{}' for '{}'",
            original_name, alias_name
        );
        assert_eq!(
            import_alias.module_name, module_name,
            "Module name should be '{}' for '{}'",
            module_name, alias_name
        );
        assert_eq!(
            import_alias.is_from_import, is_from_import,
            "is_from_import should be {} for '{}'",
            is_from_import, alias_name
        );
        assert_eq!(
            import_alias.has_explicit_alias, has_explicit_alias,
            "has_explicit_alias should be {} for '{}'",
            has_explicit_alias, alias_name
        );
    }

    // Verify that non-aliased imports were NOT collected
    let non_aliased_imports = vec!["math", "random", "datetime"];
    for import_name in non_aliased_imports {
        assert!(
            !import_aliases.contains_key(import_name),
            "Should NOT contain non-aliased import '{}'",
            import_name
        );
    }
}

/// Test that regular import aliases are correctly distinguished from "from" imports
#[test]
fn test_regular_vs_from_import_distinction() {
    let python_code = r#"
# Regular import with alias
import os as operating_system

# From import with alias
from collections import defaultdict as default_dict

# From import without alias
from json import dumps

def main():
    pass
"#;

    // Parse the Python code
    let parsed =
        ruff_python_parser::parse_module(python_code).expect("Failed to parse Python code");
    let module_ast = parsed.syntax();

    // Create AST rewriter and collect import aliases
    let mut rewriter = AstRewriter::new(10); // Python 3.10
    rewriter.collect_import_aliases(module_ast, "test_module");

    let import_aliases = rewriter.import_aliases();

    // Should have collected 3 import aliases
    assert_eq!(import_aliases.len(), 3, "Should collect 3 import aliases");

    // Check regular import alias
    assert!(import_aliases.contains_key("operating_system"));
    let regular_alias = &import_aliases["operating_system"];
    assert!(!regular_alias.is_from_import);
    assert_eq!(regular_alias.original_name, "os");
    assert_eq!(regular_alias.module_name, "os"); // For regular imports, module_name == original_name
    assert!(regular_alias.has_explicit_alias);

    // Check from import with alias
    assert!(import_aliases.contains_key("default_dict"));
    let from_alias = &import_aliases["default_dict"];
    assert!(from_alias.is_from_import);
    assert_eq!(from_alias.original_name, "defaultdict");
    assert_eq!(from_alias.module_name, "collections"); // For from imports, module_name is the source module
    assert!(from_alias.has_explicit_alias);

    // Check from import without alias
    assert!(import_aliases.contains_key("dumps"));
    let from_no_alias = &import_aliases["dumps"];
    assert!(from_no_alias.is_from_import);
    assert_eq!(from_no_alias.original_name, "dumps");
    assert_eq!(from_no_alias.module_name, "json");
    assert!(!from_no_alias.has_explicit_alias);
}

/// Test edge cases with dotted module names in regular imports
#[test]
fn test_regular_import_dotted_modules() {
    let python_code = r#"
# Complex dotted module imports with aliases
import xml.etree.ElementTree as ET
import collections.abc as abc_module
import urllib.parse as parse_utils
import email.mime.text as email_text

def main():
    pass
"#;

    // Parse the Python code
    let parsed =
        ruff_python_parser::parse_module(python_code).expect("Failed to parse Python code");
    let module_ast = parsed.syntax();

    // Create AST rewriter and collect import aliases
    let mut rewriter = AstRewriter::new(10); // Python 3.10
    rewriter.collect_import_aliases(module_ast, "test_module");

    let import_aliases = rewriter.import_aliases();

    // Should have collected 4 dotted module aliases
    assert_eq!(
        import_aliases.len(),
        4,
        "Should collect 4 dotted module aliases"
    );

    // Test each dotted module alias
    let dotted_cases = vec![
        ("ET", "xml.etree.ElementTree"),
        ("abc_module", "collections.abc"),
        ("parse_utils", "urllib.parse"),
        ("email_text", "email.mime.text"),
    ];

    for (alias_name, full_module_name) in dotted_cases {
        assert!(
            import_aliases.contains_key(alias_name),
            "Should contain dotted module alias '{}'",
            alias_name
        );

        let import_alias = &import_aliases[alias_name];

        // For regular imports, original_name and module_name should be the same
        assert_eq!(import_alias.original_name, full_module_name);
        assert_eq!(import_alias.module_name, full_module_name);
        assert_eq!(import_alias.alias_name, alias_name);
        assert!(!import_alias.is_from_import);
        assert!(import_alias.has_explicit_alias);
    }
}

/// Test that multiple aliases in a single import statement are handled correctly
#[test]
fn test_multiple_aliases_single_import() {
    let python_code = r#"
# Multiple imports with aliases in single statement - this is valid Python
import os as operating_system, sys as system_info, json as j

def main():
    pass
"#;

    // Parse the Python code
    let parsed =
        ruff_python_parser::parse_module(python_code).expect("Failed to parse Python code");
    let module_ast = parsed.syntax();

    // Create AST rewriter and collect import aliases
    let mut rewriter = AstRewriter::new(10); // Python 3.10
    rewriter.collect_import_aliases(module_ast, "test_module");

    let import_aliases = rewriter.import_aliases();

    // Should have collected all 3 aliases from the single import statement
    assert_eq!(
        import_aliases.len(),
        3,
        "Should collect 3 aliases from single import statement"
    );

    // Verify each alias
    let expected_multiple = vec![
        ("operating_system", "os"),
        ("system_info", "sys"),
        ("j", "json"),
    ];

    for (alias_name, original_name) in expected_multiple {
        assert!(
            import_aliases.contains_key(alias_name),
            "Should contain alias '{}' from multiple import",
            alias_name
        );

        let import_alias = &import_aliases[alias_name];
        assert_eq!(import_alias.original_name, original_name);
        assert_eq!(import_alias.module_name, original_name); // Same for regular imports
        assert_eq!(import_alias.alias_name, alias_name);
        assert!(!import_alias.is_from_import);
        assert!(import_alias.has_explicit_alias);
    }
}
