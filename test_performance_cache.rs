use serpen::config::Config;
use serpen::resolver::ModuleResolver;
use std::time::Instant;

fn main() {
    // Create a resolver
    let config = Config::default();
    let resolver = ModuleResolver::new(config).unwrap();

    println!("Testing virtualenv packages caching performance...");

    // First call - this should scan the filesystem and cache the result
    let start = Instant::now();
    let first_classification = resolver.classify_import("numpy");
    let first_duration = start.elapsed();
    println!(
        "First call took: {:?} -> {:?}",
        first_duration, first_classification
    );

    // Second call - this should use the cached result
    let start = Instant::now();
    let second_classification = resolver.classify_import("numpy");
    let second_duration = start.elapsed();
    println!(
        "Second call took: {:?} -> {:?}",
        second_duration, second_classification
    );

    // Third call - this should also use the cached result
    let start = Instant::now();
    let third_classification = resolver.classify_import("requests");
    let third_duration = start.elapsed();
    println!(
        "Third call took: {:?} -> {:?}",
        third_duration, third_classification
    );

    println!("Cache is working if subsequent calls are significantly faster!");
    println!(
        "First: {:?}, Second: {:?}, Third: {:?}",
        first_duration, second_duration, third_duration
    );
}
