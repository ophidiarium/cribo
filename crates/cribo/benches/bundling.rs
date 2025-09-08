use std::{fs, hint::black_box, path::Path};

use cribo::{config::Config, orchestrator::BundleOrchestrator};
use criterion::{Criterion, criterion_group, criterion_main};
use tempfile::TempDir;

/// Create a simple test project structure for benchmarking
fn create_test_project(dir: &Path) -> std::io::Result<()> {
    // Create main.py
    fs::write(
        dir.join("main.py"),
        r#"#!/usr/bin/env python3
from utils.helpers import process_data
from models.user import User

def main():
    user = User("Alice", "alice@example.com")
    data = process_data(user.to_dict())
    print(f"Processed: {data}")

if __name__ == "__main__":
    main()
"#,
    )?;

    // Create utils directory and helpers.py
    fs::create_dir_all(dir.join("utils"))?;
    fs::write(dir.join("utils").join("__init__.py"), "# Utils package")?;
    fs::write(
        dir.join("utils").join("helpers.py"),
        r#"import json
from typing import Dict, Any

def process_data(data: Dict[str, Any]) -> str:
    """Process user data and return JSON string."""
    processed = {
        "user": data,
        "timestamp": "2024-01-01T00:00:00Z",
        "status": "processed"
    }
    return json.dumps(processed, indent=2)

def validate_email(email: str) -> bool:
    """Simple email validation."""
    return "@" in email and "." in email.split("@")[1]
"#,
    )?;

    // Create models directory and user.py
    fs::create_dir_all(dir.join("models"))?;
    fs::write(dir.join("models").join("__init__.py"), "# Models package")?;
    fs::write(
        dir.join("models").join("user.py"),
        r#"from dataclasses import dataclass
from typing import Dict, Any

@dataclass
class User:
    name: str
    email: str

    def to_dict(self) -> Dict[str, Any]:
        return {
            "name": self.name,
            "email": self.email
        }

    def __str__(self) -> str:
        return f"User(name={self.name}, email={self.email})"
"#,
    )?;

    Ok(())
}

/// Benchmark the full bundling process
fn benchmark_bundling(c: &mut Criterion) {
    c.bench_function("bundle_simple_project", |b| {
        b.iter_with_setup(
            || {
                // Setup: Create temp directory with test project
                let temp_dir = TempDir::new().expect("Failed to create temp dir");
                create_test_project(temp_dir.path()).expect("Failed to create test project");

                let entry_path = temp_dir.path().join("main.py");
                let output_path = temp_dir.path().join("bundle.py");

                let mut config = Config::default();
                config.src.push(temp_dir.path().to_path_buf());

                (temp_dir, entry_path, output_path, config)
            },
            |(temp_dir, entry_path, output_path, config)| {
                // Benchmark: Bundle the project
                let mut bundler = BundleOrchestrator::new(config);
                bundler
                    .bundle(black_box(&entry_path), black_box(&output_path), false)
                    .expect("Bundling should succeed");

                // Keep temp_dir alive until benchmark completes
                drop(temp_dir);
            },
        );
    });
}

/// Benchmark module resolution
fn benchmark_module_resolution(c: &mut Criterion) {
    use cribo::resolver::ModuleResolver;

    c.bench_function("resolve_module_path", |b| {
        // Setup resolver once
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        create_test_project(temp_dir.path()).expect("Failed to create test project");

        let mut config = Config::default();
        config.src.push(temp_dir.path().to_path_buf());

        let resolver = ModuleResolver::new(config);

        b.iter(|| {
            // Benchmark module resolution
            let _ = resolver.resolve_module_path(black_box("utils.helpers"));
            let _ = resolver.resolve_module_path(black_box("models.user"));
            let _ = resolver.resolve_module_path(black_box("json"));
        });
    });
}

/// Benchmark dependency graph construction
fn benchmark_dependency_graph(c: &mut Criterion) {
    use std::path::PathBuf;

    use cribo::{cribo_graph::CriboGraph, resolver::ModuleId};

    c.bench_function("build_dependency_graph", |b| {
        b.iter(|| {
            let mut graph = CriboGraph::new();

            // Add modules
            let main_id = graph.add_module(
                ModuleId::new(0),
                "main".to_string(),
                &PathBuf::from("main.py"),
            );
            let utils_id = graph.add_module(
                ModuleId::new(1),
                "utils.helpers".to_string(),
                &PathBuf::from("utils/helpers.py"),
            );
            let models_id = graph.add_module(
                ModuleId::new(2),
                "models.user".to_string(),
                &PathBuf::from("models/user.py"),
            );

            // Add dependencies - main depends on utils and models
            graph.add_module_dependency(main_id, utils_id);
            graph.add_module_dependency(main_id, models_id);

            // Topological sort
            let _ = graph.topological_sort();
        });
    });
}

criterion_group!(
    benches,
    benchmark_bundling,
    benchmark_module_resolution,
    benchmark_dependency_graph
);
criterion_main!(benches);
