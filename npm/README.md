# Serpen npm Publishing Setup

This directory contains the npm publishing infrastructure for Serpen, allowing distribution of the Rust CLI binary via npm packages.

## Architecture Overview

The npm publishing system follows the pattern used by projects like `@rspack/core`, `esbuild`, and `@sentry/cli`:

1. **Base Package** (`npm/cribo/`): The main package users install (`npm install cribo`)
2. **Platform Packages**: Auto-generated packages containing binaries for specific platforms
3. **Launcher Script**: Node.js script that detects platform and executes the correct binary

## Package Structure

```
npm/
├── cribo/                    # Base npm package
│   ├── package.json          # Main package metadata
│   ├── bin/cribo.js         # Node.js launcher script
│   └── README.md             # User documentation
├── package.json.tmpl         # Template for platform packages
└── README.md                 # This file
```

## Platform Support

The system builds and publishes packages for these platforms:

- **Linux**: x64 and ARM64 (both glibc and musl variants)
- **macOS**: x64 and ARM64 (Intel and Apple Silicon)
- **Windows**: x64 and ARM64

## Scripts

### `scripts/build-npm-binaries.sh`

Builds Rust binaries for all supported platforms using cross-compilation.

```bash
# Build all platforms
./scripts/build-npm-binaries.sh

# Custom output directory
./scripts/build-npm-binaries.sh --output-dir ./my-binaries

# Development build
./scripts/build-npm-binaries.sh --profile dev
```

**Features:**

- Uses `cross` for cross-compilation when needed
- Automatically installs required Rust targets
- Validates binary output
- Colored output with progress indicators

### `scripts/generate-npm-packages.js`

Generates npm packages from the template for each platform.

```bash
# Generate packages for version 0.3.0
node scripts/generate-npm-packages.js 0.3.0 ./npm-dist ./target/npm-binaries
```

**What it does:**

- Creates a directory for each platform package
- Generates `package.json` from template
- Copies the correct binary
- Creates platform-specific README
- Sets proper file permissions

### `scripts/publish-npm.js`

Publishes all npm packages (platform packages first, then base package).

```bash
# Publish to npm
node scripts/publish-npm.js 0.3.0 ./npm-dist

# Dry run (test without publishing)
node scripts/publish-npm.js 0.3.0 ./npm-dist --dry-run

# Publish with custom tag
node scripts/publish-npm.js 0.3.0 ./npm-dist --tag beta
```

**Safety features:**

- Publishes platform packages before base package
- Skips existing versions
- Validates package contents
- Updates version numbers automatically

### `scripts/test-npm-package.js`

Tests the npm package locally before publishing.

```bash
# Test the local package
node scripts/test-npm-package.js
```

**Test coverage:**

- Installs package in temporary directory
- Tests `npx cribo --help`
- Verifies correct platform package installation
- Validates launcher script functionality

## GitHub Actions Integration

The npm publishing is integrated into `.github/workflows/release.yml`:

1. **build-npm-binaries**: Builds binaries for all platforms
2. **publish-to-npm**: Publishes packages to npm registry

## Local Development

### Testing Changes

1. Build binaries:
   ```bash
   ./scripts/build-npm-binaries.sh
   ```

2. Generate packages:
   ```bash
   node scripts/generate-npm-packages.js "0.3.0-dev" ./npm-dist ./target/npm-binaries
   ```

3. Test locally:
   ```bash
   node scripts/test-npm-package.js
   ```

### Manual Publishing

1. Set npm token:
   ```bash
   export NPM_TOKEN="your-npm-token"
   ```

2. Publish with dry run:
   ```bash
   node scripts/publish-npm.js "0.3.0" ./npm-dist --dry-run
   ```

3. Publish for real:
   ```bash
   node scripts/publish-npm.js "0.3.0" ./npm-dist
   ```

## User Experience

Users can install and use Serpen via npm:

```bash
# Global installation
npm install -g cribo
cribo --help

# One-time use
npx cribo --help

# In project
npm install cribo
npx cribo --help
```

The system automatically:

- Downloads only the correct binary for the user's platform
- Provides helpful error messages if binaries are missing
- Falls back gracefully when optional dependencies are disabled

## Platform Detection

The launcher script (`npm/cribo/bin/cribo.js`) detects:

- **Operating System**: Linux, macOS, Windows
- **Architecture**: x64, ARM64, x86 (Windows only)
- **Libc variant**: glibc vs musl (Linux only)

## Troubleshooting

### Binary Not Found

If users see "Could not find Serpen binary", they should:

1. Reinstall with optional dependencies:
   ```bash
   npm install cribo
   ```

2. Check if optional dependencies are enabled:
   ```bash
   npm install --include=optional
   ```

### Cross-compilation Issues

If cross-compilation fails:

1. Install cross:
   ```bash
   cargo install cross --git https://github.com/cross-rs/cross
   ```

2. Install required targets:
   ```bash
   rustup target add aarch64-unknown-linux-gnu
   # ... other targets
   ```

### Publishing Errors

Common issues:

- **Version already exists**: Use `--dry-run` first, or increment version
- **Authentication**: Set `NPM_TOKEN` environment variable
- **Platform mismatch**: Ensure all platform packages publish before base package

## Version Management

Versions are automatically synchronized:

- Base package version matches Cargo.toml
- All platform packages use the same version
- optionalDependencies are updated automatically

The GitHub Actions workflow extracts version from git tags (`v1.2.3`) and updates all packages accordingly.
