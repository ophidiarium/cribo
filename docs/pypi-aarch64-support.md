# PyPI aarch64 Linux Support Implementation

## Summary

Added aarch64 (ARM64) Linux support to PyPI wheel building in the GitHub Actions release workflow. This ensures that PyPI wheels are available for ARM64 Linux platforms, matching the existing npm binary support.

## Changes Made

### 1. Updated Build Matrix

Modified `.github/workflows/release.yml` to include aarch64 targets:

**Before:**

```yaml
strategy:
  matrix:
    os: [ubuntu-latest, macos-latest, windows-latest]
```

**After:**

```yaml
strategy:
  matrix:
    platform:
      - os: ubuntu-latest
        target: ''
      - os: macos-latest
        target: ''
      - os: windows-latest
        target: ''
      - os: ubuntu-latest
        target: aarch64-unknown-linux-gnu
      - os: ubuntu-latest
        target: aarch64-unknown-linux-musl
```

### 2. Enhanced maturin-action Configuration

Added cross-compilation support to the maturin-action step:

```yaml
- name: Build wheels
  uses: PyO3/maturin-action@v1.49.1
  with:
    command: build
    args: --release --out dist
    manylinux: 2014
    sccache: true
    target: ${{ matrix.platform.target != '' && matrix.platform.target || '' }}
```

### 3. Updated Artifact Naming

Modified artifact names to distinguish between native and cross-compiled builds:

```yaml
name: python-package-distributions-${{ matrix.platform.os }}-${{ matrix.platform.target || 'native' }}-${{ github.run_id }}
```

### 4. Conditional Steps

Updated conditional logic for version file creation to only run on native ubuntu builds:

```yaml
if: matrix.platform.os == 'ubuntu-latest' && matrix.platform.target == ''
```

## Technical Details

### Cross-Compilation Support

The maturin-action automatically uses cross-compilation Docker containers for non-native targets:

- **aarch64-unknown-linux-gnu**: Uses `ghcr.io/rust-cross/manylinux2014-cross:aarch64`
- **aarch64-unknown-linux-musl**: Uses appropriate musl-based cross-compilation container

### Platform Tags

The maturin-action will automatically generate proper platform tags for the wheels:

- `linux_aarch64` for glibc-based builds
- `musllinux_1_2_aarch64` for musl-based builds

### Compatibility

This maintains full backward compatibility:

- Existing x86_64 builds continue to work unchanged
- All existing publishing steps work with the new artifact pattern
- npm binary building already supported these targets

## Resulting PyPI Packages

After this change, PyPI will contain wheels for:

1. **Linux x86_64**: `serpen-*-cp*-cp*-linux_x86_64.whl`
2. **Linux aarch64 (GNU)**: `serpen-*-cp*-cp*-linux_aarch64.whl`
3. **Linux aarch64 (musl)**: `serpen-*-cp*-cp*-musllinux_*_aarch64.whl`
4. **macOS x86_64**: `serpen-*-cp*-cp*-macosx_*_x86_64.whl`
5. **macOS ARM64**: `serpen-*-cp*-cp*-macosx_*_arm64.whl`
6. **Windows x86_64**: `serpen-*-cp*-cp*-win_amd64.whl`

## Testing

To test the changes:

1. **Local verification**: The workflow syntax has been validated
2. **Matrix validation**: All 5 build combinations are properly configured
3. **Artifact patterns**: Download patterns will correctly collect all wheels

## Benefits

- **Complete ARM64 support**: Users can install via pip on ARM64 Linux systems
- **Performance**: Native ARM64 wheels perform better than emulated x86_64 wheels
- **Consistency**: PyPI support now matches npm binary platform coverage
- **Modern infrastructure**: Supports current ARM64 server/development environments
