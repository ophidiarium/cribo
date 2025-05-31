use serpen::bundler::Bundler;
use serpen::config::Config;
use std::path::PathBuf;

fn main() {
    let mut config = Config::default();
    config.src = vec![PathBuf::from("tests/fixtures/simple_project")];
    let bundler = Bundler::new(config);

    let main_path = PathBuf::from("tests/fixtures/simple_project/main.py");
    if let Ok(imports) = bundler.extract_imports(&main_path) {
        println!("Extracted imports from main.py: {:?}", imports);
    } else {
        println!("Failed to extract imports from main.py");
    }
}
