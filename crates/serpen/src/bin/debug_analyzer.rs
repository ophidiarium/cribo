use serpen::unused_imports_simple::UnusedImportAnalyzer;
use std::fs;

fn main() -> anyhow::Result<()> {
    let mut analyzer = UnusedImportAnalyzer::new();
    let content = fs::read_to_string("/tmp/test_trim.py")?;

    println!("File content:");
    println!("{}", content);
    println!("\n--- Analysis ---");

    let unused_imports = analyzer.analyze_file(&content)?;

    println!("Found {} unused imports:", unused_imports.len());
    for import in &unused_imports {
        println!("  - {} ({})", import.name, import.qualified_name);
    }

    Ok(())
}
