---
applyTo: "**/*.rs"
---

# Project coding standards for Rust

Apply the [general coding guidelines](./general-coding.instructions.md) to all code.

## Rust Guidelines

- Use Rust for all new code
- Use Rust 2024 edition
- Follow functional programming principles where possible
- Ensure usage of proper error handling, using `Option` or `Result` types where appropriate. Utilize a custom error type for the project when reasonable.
- Ensure functions are documented with comments that abide by Rust's documentation standards
- Ensure that functions are tested in a way that is consistent with the rest of the codebase
- Ensure that functions are small and focused on a single task
- Use `async`/`await` for asynchronous code
- Use `serde` and `postcard` for serialization/deserialization of data structures
- `alloc` is available for heap allocation, but use it sparingly
- Ensure that any feature gates that are added are added to the Cargo.toml and documented
- Ensure that any dependencies that are added are added to the Cargo.toml and documented
- When making asynchronous functions, use `async fn` and `await` for calling other asynchronous functions, do not return a `Future` directly unless absolutely necessary
- When reviewing Rust code, always make sure there is enough context to ensure the borrow checker is satisfied

## Related Rust Processes

- Use `clippy` for linting and code quality checks
- Use `rustfmt` for formatting
- Use `cargo test` for unit tests and integration tests
- Use `cargo doc` for generating documentation
