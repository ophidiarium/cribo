# PyPI Wheel Tag Ordering Fix

## Issue Description

The Cribo project was experiencing PyPI publishing failures due to wheel filename tag ordering that violates PEP 425 requirements. The issue manifested as:

- **Built wheel filename**: `manylinux2014_aarch64.manylinux_2_17_aarch64.whl`
- **Expected by attestation**: `manylinux_2_17_aarch64.manylinux2014_aarch64.whl`

This mismatch caused PyPI's attestation verification to fail because the platform tags were not sorted according to PEP 425 specifications.

> **⚠️ Current Status**: PyPI attestations are temporarily disabled while we implement a source-level fix. See [PyPI Attestations Temporary Disable](pypi-attestations-temporary-disable.md) for details.

## Root Cause

The issue stems from maturin/auditwheel not properly sorting compressed tag sets in wheel filenames. According to PEP 425, when multiple platform tags are present, they must be sorted in lexicographical order.

## Solution

### 1. Maturin Version Update

Updated `pyproject.toml` to require maturin >= 1.7, which includes fixes for tag ordering:

```toml
[build-system]
requires = ["maturin>=1.7,<2.0"]
build-backend = "maturin"
```

### 2. Explicit Maturin Version in GitHub Actions

Added explicit maturin version in the build workflow:

```yaml
- name: Build wheels
  uses: PyO3/maturin-action@aef21716ff3dcae8a1c301d23ec3e4446972a6e3
  with:
    command: build
    args: --release --out dist
    manylinux: 2014
    sccache: true
    target: ${{ matrix.platform.rust_target }}
    maturin-version: '1.7.4'
```

### 3. Post-Build Tag Ordering Fix

Added a post-processing step to ensure PEP 425 compliance:

```bash
# Fix wheel filenames to ensure platform tags are properly sorted per PEP 425
for wheel in dist/*.whl; do
  if [[ -f "$wheel" ]]; then
    filename=$(basename "$wheel")
    
    # Fix aarch64 tag ordering
    if [[ "$filename" =~ manylinux2014_aarch64\.manylinux_2_17_aarch64 ]]; then
      corrected_filename=$(echo "$filename" | sed 's/manylinux2014_aarch64\.manylinux_2_17_aarch64/manylinux_2_17_aarch64.manylinux2014_aarch64/')
      mv "$wheel" "dist/$corrected_filename"
    fi
    
    # Fix x86_64 tag ordering if needed
    if [[ "$filename" =~ manylinux2014_x86_64\.manylinux_2_17_x86_64 ]]; then
      corrected_filename=$(echo "$filename" | sed 's/manylinux2014_x86_64\.manylinux_2_17_x86_64/manylinux_2_17_x86_64.manylinux2014_x86_64/')
      mv "$wheel" "dist/$corrected_filename"
    fi
  fi
done
```

## Recent Improvements (2025-06-03)

### Maturin CI Pattern Adoption

Following analysis of the official maturin-generated CI workflow, we've adopted several proven patterns:

- **Conditional sccache**: Disabled for release builds to ensure clean compilation
- **Enhanced manylinux**: Using `manylinux: auto` for optimal compatibility detection
- **Centralized SLSA attestations**: Modern build provenance with `actions/attest-build-provenance@v2`
- **Source distribution**: Added dedicated sdist job following maturin patterns
- **Improved artifact naming**: Cleaner organization with `wheels-*` patterns

See [maturin-ci-pattern-adoption.md](./maturin-ci-pattern-adoption.md) for detailed implementation.

## PEP 425 Background

[PEP 425](https://peps.python.org/pep-0425/) specifies the format for Python wheel filenames:

```
{distribution}-{version}(-{build tag})?-{python tag}-{abi tag}-{platform tag}.whl
```

For compressed tag sets (multiple tags separated by dots), the tags must be sorted in lexicographical order. This ensures:

1. **Deterministic naming**: Same wheel content produces same filename
2. **Attestation verification**: PyPI can verify wheel integrity
3. **Package resolution**: pip can correctly identify compatible wheels

## Verification

After applying the fix, wheel filenames will be correctly ordered:

- ✅ `cribo-1.0.0-py3-none-manylinux_2_17_aarch64.manylinux2014_aarch64.whl`
- ❌ `cribo-1.0.0-py3-none-manylinux2014_aarch64.manylinux_2_17_aarch64.whl`

## Testing

The fix logic has been tested with the included test script:

```bash
./scripts/test-wheel-tag-fix.sh
```

This creates test wheels with problematic names and verifies the fix correctly reorders the tags.

## Related Issues

- [pypa/auditwheel#583](https://github.com/pypa/auditwheel/issues/583) - Original issue report
- [pypa/auditwheel#584](https://github.com/pypa/auditwheel/pull/584) - Fix implementation
- PEP 425 - Compatibility Tags for Built Distributions

## Security Benefits

This fix ensures:

1. **PyPI Attestation Support**: Wheels can be properly verified against SLSA attestations
2. **Supply Chain Security**: Provenance information is correctly associated with packages
3. **Trusted Publishing**: Integration with GitHub Actions OIDC for secure publishing
