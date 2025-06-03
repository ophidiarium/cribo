# Maturin CI Pattern Adoption

## Overview

This document details the comprehensive improvements made to our `release.yml` workflow by adopting proven patterns from the maturin-generated CI workflow (`CI-1.yml`). These changes enhance build reliability, modernize our SLSA attestation approach, and follow established best practices from the PyO3/maturin ecosystem.

## Key Improvements Implemented

### 1. **Conditional sccache for Clean Release Builds**

**Pattern Adopted:**

```yaml
sccache: ${{ !startsWith(github.ref, 'refs/tags/') }}
```

**Rationale:**

- Maturin CI disables sccache for release builds to ensure completely clean, reproducible builds
- Development builds still benefit from caching for faster iteration
- Prevents potential cache-related issues in production releases

**Impact:** More reliable release builds with guaranteed clean compilation.

### 2. **Improved Artifact Organization**

- **Old Pattern**: `python-package-distributions-{target}-{run_id}`
- **New Pattern**: `wheels-{target}-{run_id}`, `npm-binary-{target}-{run_id}`
- **Benefit**: Cleaner naming scheme following maturin conventions
- **Impact**: Easier to understand and manage artifacts

### 3. **Centralized SLSA Build Provenance**

- **Added**: `generate-attestations` job using `actions/attest-build-provenance@v2`
- **Permissions**:
  - `id-token: write` - For signing artifacts
  - `contents: write` - For uploading release artifacts
  - `attestations: write` - For generating attestations
- **Benefit**: Modern SLSA (Supply-chain Levels for Software Artifacts) compliance
- **Coverage**: Covers all build artifacts (wheels, npm packages, binaries)

### 4. **Enhanced manylinux Configuration**

- **Old**: `args: --release --out dist --compatibility manylinux_2_17`
- **New**: `args: --release --out dist` with `manylinux: auto`
- **Benefit**: Automatic manylinux detection for optimal compatibility
- **Rationale**: Follows maturin CI pattern for flexible platform support

### 5. **Source Distribution (sdist) Support**

- **Added**: Dedicated `build-sdist` job
- **Pattern**: Follows maturin CI structure with separate sdist build
- **Benefit**: Complete package distribution including source packages
- **Integration**: Included in publishing and attestation workflows

### 6. **Precise Permissions Scoping**

- **Pattern**: Job-level permissions with minimal required scopes
- **Benefits**:
  - **Security**: Principle of least privilege
  - **Compliance**: OIDC token requirements for provenance
  - **Clarity**: Explicit permission requirements per job

### 7. **Temporary PyPI Attestation Management**

- **Status**: Individual PyPI attestations temporarily disabled
- **Reason**: Prevents conflicts during wheel filename fixes
- **Plan**: Re-enable once source-level wheel tag ordering is resolved
- **Alternative**: Centralized SLSA attestations provide provenance coverage

## Architecture Changes

### Before: Distributed Release Pattern

```yaml
build-job:
  - build artifacts
  - upload artifacts
  - publish directly
```

### After: Centralized Release Pattern

```yaml
build-job:
  - build artifacts
  - upload artifacts

sdist-job:
  - build source distribution
  - upload sdist

publish-jobs:
  - download artifacts
  - publish to registries

attestation-job:
  - download all artifacts
  - generate SLSA provenance
```

## Security Improvements

### SLSA Build Provenance

- **Level**: SLSA Level 2 compliance
- **Signing**: Sigstore-based attestations
- **Coverage**: All build artifacts
- **Verification**: Enables downstream verification of build integrity

### OIDC Token Security

- **GitHub Actions**: Native OIDC integration
- **npm**: Provenance support with `--provenance` flag
- **PyPI**: Trusted publishing with attestations
- **Benefits**: No long-lived secrets, cryptographic verification

## Performance Optimizations

### Build Cache Strategy

- **Development**: sccache enabled for fast iteration
- **Release**: sccache disabled for clean builds
- **Rationale**: Balances development speed with release reliability

### Artifact Management

- **Retention**: 7-day cleanup policy
- **Organization**: Platform-specific naming
- **Efficiency**: Reduced artifact storage costs

## Compliance Features

### Supply Chain Security

- **SLSA Provenance**: Build process attestation
- **Sigstore Integration**: Keyless signing infrastructure
- **Verification**: End-to-end artifact integrity

### Auditability

- **Build Metadata**: Version tracking in JSON format
- **Artifact Traceability**: Run ID correlation
- **Process Documentation**: Explicit workflow steps

## Best Practices Adopted

1. **Separation of Concerns**: Build, package, publish, and attest as separate jobs
2. **Conditional Logic**: Environment-aware configuration
3. **Error Handling**: Proper validation and early termination
4. **Documentation**: Inline comments explaining patterns
5. **Backwards Compatibility**: Maintains existing functionality

## Future Enhancements

### Potential Zig Integration

- **Benefit**: More efficient cross-compilation for some targets
- **Consideration**: Evaluate impact on build times and reliability

### Enhanced Matrix Strategy

- **Pattern**: Separate platform categories (linux/musllinux/windows/macos)
- **Benefit**: More granular control and potential parallelization

### Advanced Caching

- **Rust Dependencies**: Enhanced Cargo cache strategies
- **Build Artifacts**: Cross-job artifact reuse optimization

## Migration Impact

### Immediate Benefits

- ✅ Cleaner release builds
- ✅ Modern SLSA compliance
- ✅ Better artifact organization
- ✅ Enhanced security posture

### Zero Downtime

- ✅ Backwards compatible artifact patterns
- ✅ Existing functionality preserved
- ✅ Gradual enhancement approach

This implementation successfully adopts maturin's proven CI patterns while maintaining our existing functionality and preparing for future enhancements.
