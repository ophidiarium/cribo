use serpen::unused_imports_simple::UnusedImportAnalyzer;

/// Test that __future__ imports are skipped and not reported as unused
#[test]
fn test_future_import_skipped() {
    let source = r#"
from __future__ import annotations, print_function
import sys
"#;
    let mut analyzer = UnusedImportAnalyzer::new();
    let unused = analyzer
        .analyze_file(source)
        .expect("Failed to analyze source");

    // Should not flag any __future__ imports
    assert!(
        unused.iter().all(|imp| imp.qualified_name != "__future__"),
        "__future__ imports should be skipped"
    );

    // Should flag sys as unused
    let sys_unused: Vec<_> = unused
        .iter()
        .filter(|imp| imp.qualified_name == "sys")
        .collect();
    assert_eq!(
        sys_unused.len(),
        1,
        "sys should be reported as one unused import"
    );
}

/// Test that when only __future__ imports are present, no unused imports are reported
#[test]
fn test_only_future_imports() {
    let source = "from __future__ import annotations";
    let mut analyzer = UnusedImportAnalyzer::new();
    let unused = analyzer
        .analyze_file(source)
        .expect("Failed to analyze source");
    assert!(unused.is_empty(), "No unused imports should be reported");
}
