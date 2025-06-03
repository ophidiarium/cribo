# Serpen

Python source bundler that produces a single .py file from multi-module projects.

## Installation

```bash
npm install -g serpen
# or
npx serpen [options]
```

## Usage

```bash
serpen --help
```

## Platform Support

This package automatically installs the correct binary for your platform:

- Linux (x64, ARM64) - both glibc and musl variants
- macOS (x64, ARM64)
- Windows (x64, x86)

## Requirements

- Node.js 18.0.0 or higher (for npm installation only)
- The Serpen binary is a standalone Rust executable

## More Information

For detailed documentation, visit: https://github.com/tinovyatkin/serpen
