---
applyTo: "**/*.rs"
---

# Project coding standards for Rust

Apply the [general coding guidelines](./general-coding.instructions.md) to all code.

## Rust Guidelines

- Use idiomatic, modern Rust (2024 edition or later).
- Use strong typing and Rust’s safety/concurrency principles throughout.
- Ensure usage of proper error handling, using `Option` or `Result` types where appropriate. Utilize a custom error type for the project when reasonable.
- Ensure functions are documented with comments that abide by Rust's documentation standards
- Ensure that functions are tested in a way that is consistent with the rest of the codebase
- Write testable, extensible code; prefer pure functions where possible.
- Use `async`/`await` for asynchronous code
- Use `serde` and `postcard` for serialization/deserialization of data structures
- `alloc` is available for heap allocation, but use it sparingly
- Ensure that any feature gates that are added are added to the Cargo.toml and documented
- Ensure that any dependencies that are added are added to the Cargo.toml and documented
- When making asynchronous functions, use `async fn` and `await` for calling other asynchronous functions, do not return a `Future` directly unless absolutely necessary
- When reviewing Rust code, always make sure there is enough context to ensure the borrow checker is satisfied

## Logging Guidelines

- Always use structured logging instead of `println!` for debug output: `use log::{debug, info, warn, error};`
- Use appropriate log levels:
  - `debug!()` for detailed diagnostic information useful during development
  - `info!()` for general information about program execution
  - `warn!()` for potentially problematic situations
  - `error!()` for error conditions that should be addressed
- If debug logging was essential to find a bug in the codebase, that logging should be kept in the codebase at the appropriate log level to aid future debugging
- Avoid temporary `println!` statements - replace them with proper logging before committing code
- Use structured logging with context where helpful: `debug!("Processing file: {}", file_path)`

## Related Rust Processes

- Use `cargo clippy` for linting and code quality checks
- Use `cargo fmt` for formatting
- Use `cargo test` for unit tests and integration tests
- Use `cargo doc` for generating documentation
