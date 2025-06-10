use cribo::config::Config;
use cribo::orchestrator::BundleOrchestrator;
use criterion::{Criterion, criterion_group, criterion_main};
use std::fs;
use std::hint::black_box;
use std::path::Path;
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

        let mut resolver = ModuleResolver::new(config).expect("Failed to create resolver");

        b.iter(|| {
            // Benchmark module resolution
            let _ = resolver.resolve_module_path(black_box("utils.helpers"));
            let _ = resolver.resolve_module_path(black_box("models.user"));
            let _ = resolver.resolve_module_path(black_box("json"));
        });
    });
}

/// Benchmark AST parsing
fn benchmark_ast_parsing(c: &mut Criterion) {
    use ruff_python_parser;

    let source = r#"
import os
import sys
from typing import List, Dict, Optional
from dataclasses import dataclass

@dataclass
class Config:
    name: str
    value: Optional[str] = None
    
    def validate(self) -> bool:
        return bool(self.name)

def process_configs(configs: List[Config]) -> Dict[str, str]:
    result = {}
    for config in configs:
        if config.validate():
            result[config.name] = config.value or "default"
    return result

def main():
    configs = [
        Config("debug", "true"),
        Config("verbose"),
        Config("output", "/tmp/out.txt")
    ]
    
    processed = process_configs(configs)
    for name, value in processed.items():
        print(f"{name} = {value}")

if __name__ == "__main__":
    main()
"#;

    c.bench_function("parse_python_ast", |b| {
        b.iter(|| {
            let _ = ruff_python_parser::parse_module(black_box(source))
                .expect("Parsing should succeed");
        });
    });
}

/// Benchmark import extraction
fn benchmark_import_extraction(c: &mut Criterion) {
    use cribo::orchestrator::BundleOrchestrator;

    c.bench_function("extract_imports", |b| {
        b.iter_with_setup(
            || {
                // Setup: Create a Python file with various import patterns
                let temp_dir = TempDir::new().expect("Failed to create temp dir");
                let test_file = temp_dir.path().join("test_imports.py");

                fs::write(
                    &test_file,
                    r#"
import os
import sys
from pathlib import Path
from typing import List, Dict, Optional, Union
from collections import defaultdict, Counter
import json
import math
from datetime import datetime, timedelta
from . import local_module
from ..parent import parent_module
from ...grandparent import grandparent_module
import xml.etree.ElementTree as ET
from urllib.parse import urlparse, urljoin
"#,
                )
                .expect("Failed to write test file");

                (temp_dir, test_file)
            },
            |(temp_dir, test_file)| {
                // Benchmark: Extract imports
                let bundler = BundleOrchestrator::new(Config::default());
                let imports = bundler
                    .extract_imports(black_box(&test_file), None)
                    .expect("Import extraction should succeed");

                // Ensure we actually extracted imports
                assert!(!imports.is_empty());

                // Keep temp_dir alive
                drop(temp_dir);
            },
        );
    });
}

/// Benchmark dependency graph construction
fn benchmark_dependency_graph(c: &mut Criterion) {
    use cribo::dependency_graph::DependencyGraph;
    use cribo::dependency_graph::ModuleNode;
    use std::path::PathBuf;

    c.bench_function("build_dependency_graph", |b| {
        b.iter(|| {
            let mut graph = DependencyGraph::new();

            // Add modules
            let modules = vec![
                ModuleNode {
                    name: "main".to_string(),
                    path: PathBuf::from("main.py"),
                    imports: vec!["utils.helpers".to_string(), "models.user".to_string()],
                },
                ModuleNode {
                    name: "utils.helpers".to_string(),
                    path: PathBuf::from("utils/helpers.py"),
                    imports: vec!["json".to_string()],
                },
                ModuleNode {
                    name: "models.user".to_string(),
                    path: PathBuf::from("models/user.py"),
                    imports: vec!["dataclasses".to_string(), "typing".to_string()],
                },
            ];

            for module in &modules {
                graph.add_module(module.clone());
            }

            // Add dependencies
            let _ = graph.add_dependency("utils.helpers", "main");
            let _ = graph.add_dependency("models.user", "main");

            // Topological sort
            let _ = graph.topological_sort();
        });
    });
}

criterion_group!(
    benches,
    benchmark_bundling,
    benchmark_module_resolution,
    benchmark_ast_parsing,
    benchmark_import_extraction,
    benchmark_dependency_graph
);
criterion_main!(benches);
