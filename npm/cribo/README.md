# Cribo

Python source bundler that produces a single .py file from multi-module projects.

## Installation

```bash
npm install -g cribo
# or
npx cribo [options]
```

## Usage

```bash
cribo --help
```

## Platform Support

This package automatically installs the correct binary for your platform:

- Linux (x64, ARM64) - both glibc and musl variants
- macOS (x64, ARM64)
- Windows (x64, x86)

## Requirements

- Node.js 18.0.0 or higher (for npm installation only)
- The Cribo binary is a standalone Rust executable

## More Information

For detailed documentation, visit: [Cribo GitHub repository](https://github.com/ophidiarium/cribo)
