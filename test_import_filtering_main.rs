use serpen::{config::Config, emit::CodeEmitter, resolver::ModuleResolver};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    // Initialize components
    let config = Config {
        src: vec![PathBuf::from(".")],
        ..Default::default()
    };

    let resolver = ModuleResolver::new(config)?;
    let mut emitter = CodeEmitter::new(resolver, false, false);

    // Test file path
    let test_file = PathBuf::from("test_import_filtering.py");

    // Process the test file
    let result = emitter.process_module_file(&test_file, "test_module")?;

    println!("Processed file content:");
    println!("{}", result);

    Ok(())
}
