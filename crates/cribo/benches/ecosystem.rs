use std::{fs, path::Path, time::Duration};

use cribo::{config::Config, orchestrator::BundleOrchestrator};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use tempfile::TempDir;

/// Bundle an ecosystem package and measure performance
fn bundle_ecosystem_package(package_name: &str) -> Result<Duration, Box<dyn std::error::Error>> {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get parent of manifest dir")
        .parent()
        .expect("Failed to get workspace root");

    // Determine entry point based on package
    let entry_point = match package_name {
        "requests" => workspace_root.join("ecosystem/packages/requests/src/requests/__init__.py"),
        _ => panic!("Unknown package: {package_name}"),
    };

    // Create temporary output directory
    let temp_dir = TempDir::new()?;
    let output_path = temp_dir.path().join(format!("{package_name}_bundled.py"));

    // Create config with src directory
    let mut config = Config::default();
    config
        .src
        .push(workspace_root.join("ecosystem/packages/requests/src"));
    config.tree_shake = false; // Disable tree-shaking for ecosystem packages for now

    // Note: Criterion will handle the timing measurement
    // We don't use Instant::now() here due to determinism requirements

    let mut orchestrator = BundleOrchestrator::new(config);
    orchestrator
        .bundle(&entry_point, &output_path, false)
        .expect("Bundling should succeed");

    // Return a dummy duration - Criterion handles actual measurement
    let duration = Duration::from_secs(0);

    // Verify output was created
    if !output_path.exists() {
        return Err("Bundle output not created".into());
    }

    // Get bundle size for info
    let bundle_size = fs::metadata(&output_path)?.len();
    eprintln!("  Bundle size for {package_name}: {bundle_size} bytes");

    Ok(duration)
}

fn benchmark_ecosystem_bundling(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecosystem_bundling");

    // Configure for longer benchmarks
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    // Only test requests for now
    let packages = ["requests"];

    for package in &packages {
        group.bench_with_input(
            BenchmarkId::new("bundle", package),
            package,
            |b, &package_name| {
                b.iter_with_large_drop(|| {
                    bundle_ecosystem_package(package_name).expect("Failed to bundle package")
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, benchmark_ecosystem_bundling);
criterion_main!(benches);
