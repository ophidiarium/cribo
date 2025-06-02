# unparser

A complete unparser for rustpython-parser ASTs.

## Simple usage example

```rust
use rustpython_parser::Parse;
use rustpython_parser::ast::Suite;
use unparser::Unparser;

fn main() {
    // ...
    let unparser = Unparser::new();
    let stmts = Suite::parse(source_str, file_path);
    for stmt in &stmts {
        unparser.unparse_stmt(stmt);
    }
    let new_source = unparser.source;
    // ...
}
```

# Transformer

This crate also contains a transformer trait for easy transformation of ASTs with the possibility of removing nodes. Enable the `transformer` feature to use it. It is similar to rustpython-ast's visitor, with the difference that a visit functions always return `Option<...>` and statements/expressions are passed as mutable.
