#!/usr/bin/env python3
"""Test scenario for bundled rich library.

This script:
1. Bundles the rich library using cribo
2. Runs smoke tests using the bundled version
3. Verifies core functionality works correctly
"""

import sys
from pathlib import Path
from types import ModuleType
from typing import TYPE_CHECKING

import pytest

from .utils import run_cribo, format_bundle_size, load_bundled_module, ensure_test_directories, get_package_requirements, parse_requirements_file

# Type hint for better IDE support
if TYPE_CHECKING:
    import rich as RichType


@pytest.fixture(scope="module")
def bundled_rich():
    """Bundle the rich library and return the bundled module path."""
    # Ensure test directories exist
    tmp_dir = ensure_test_directories()

    # Paths - Updated to use Path.resolve() for robustness
    package_root = Path(__file__).resolve().parent.parent / "packages" / "rich"
    rich_init = package_root / "rich"
    bundled_output = tmp_dir / "rich_bundled.py"

    print("\n🔧 Bundling rich library...")
    result = run_cribo(
        str(rich_init),
        str(bundled_output),
        emit_requirements=True,
        # Rich uses dynamic imports extensively, but we'll test with tree-shaking enabled
    )

    assert result.returncode == 0, f"Failed to bundle rich: {result.stderr}"

    bundle_size = bundled_output.stat().st_size
    print(f"✅ Successfully bundled to {bundled_output}")
    print(f"   Bundle size: {format_bundle_size(bundle_size)}")

    # Check requirements.txt generation
    requirements_path = bundled_output.parent / "requirements.txt"
    assert requirements_path.exists(), "requirements.txt was not generated!"

    requirements_content = requirements_path.read_text().strip()
    print(f"\n📋 Generated requirements.txt at: {requirements_path}")
    print("   Content:")
    for line in requirements_content.splitlines():
        print(f"     - {line}")

    # Return path for loading
    return bundled_output


def test_bundle_generation(bundled_rich):
    """Test that the bundle is generated successfully."""
    assert bundled_rich.exists()
    assert bundled_rich.stat().st_size > 0


def test_requirements_generation(bundled_rich):
    """Test that requirements.txt is generated with expected dependencies."""
    requirements_path = bundled_rich.parent / "requirements.txt"
    assert requirements_path.exists()

    # Parse the generated requirements.txt
    found_deps = parse_requirements_file(requirements_path)

    # Get expected dependencies from pyproject.toml
    package_root = Path(__file__).resolve().parent.parent / "packages" / "rich"
    package_reqs = get_package_requirements(package_root)

    # Check for expected dependencies (both sets are already normalized)
    expected_deps = package_reqs["install_requires"]
    missing_deps = expected_deps - found_deps

    # We should detect all required dependencies
    assert not missing_deps, f"Missing required dependencies: {missing_deps}"

    # Check for optional dependencies
    optional_deps = package_reqs["extras_require"]
    detected_optional = found_deps & optional_deps

    if detected_optional:
        print(f"   ℹ️  Optional dependencies detected: {detected_optional}")


def test_bundled_module_loading(bundled_rich):
    """Test that the bundled module can be loaded and has expected top-level exports."""
    with load_bundled_module(bundled_rich, "rich_bundled") as rich:
        # Test that the top-level exports from rich are available
        # These are what you get with 'import rich' in normal Python
        assert hasattr(rich, "print"), "Missing rich.print function"
        assert hasattr(rich, "get_console"), "Missing rich.get_console function"
        assert hasattr(rich, "inspect"), "Missing rich.inspect function"

        # Note: We cannot test submodule imports like 'from rich.console import Console'
        # because the bundle is a single file, not a package structure.
        # This is a fundamental limitation of bundling.


def test_bundled_print_functionality(bundled_rich):
    """Test rich.print functionality using top-level exports."""
    with load_bundled_module(bundled_rich, "rich_bundled") as rich:
        # Use the top-level print function with StringIO
        import io

        output = io.StringIO()

        # rich.print is a top-level export that should work
        rich.print("Hello, World!", file=output)
        result = output.getvalue()

        # The output might have ANSI codes, but should contain our text
        assert "Hello, World!" in result


if __name__ == "__main__":
    # Run with pytest when executed directly
    sys.exit(pytest.main([__file__, "-v"]))
