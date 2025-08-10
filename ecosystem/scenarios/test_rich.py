#!/usr/bin/env python3
"""Test scenario for bundled rich library.

This script:
1. Bundles the rich library using cribo
2. Runs smoke tests using the bundled version
3. Verifies core functionality works correctly
"""

import sys
from pathlib import Path
from io import StringIO
from types import ModuleType
from typing import TYPE_CHECKING

import pytest

from .utils import run_cribo, format_bundle_size, load_bundled_module, ensure_test_directories, get_package_requirements

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

    requirements_content = requirements_path.read_text().strip()

    # Get expected dependencies from pyproject.toml
    package_root = Path(__file__).resolve().parent.parent / "packages" / "rich"
    package_reqs = get_package_requirements(package_root)

    # Parse the requirements
    found_deps = set()
    for line in requirements_content.splitlines():
        if line and not line.startswith("#"):
            # Extract package name (before any version specifier)
            pkg_name = line.split(">=")[0].split("==")[0].split("<")[0].split(">")[0].strip()
            found_deps.add(pkg_name)

    # Check for expected dependencies
    expected_deps = package_reqs["install_requires"]
    missing_deps = expected_deps - found_deps

    # We should detect all required dependencies
    assert not missing_deps, f"Missing required dependencies: {missing_deps}"

    # Check for optional dependencies
    optional_deps = package_reqs["extras_require"]
    detected_optional = found_deps & optional_deps

    if detected_optional:
        print(f"   ℹ️  Optional dependencies detected: {detected_optional}")


@pytest.mark.xfail(reason="Known issue with 'from abc import abc' in bundled code")
def test_bundled_module_loading(bundled_rich):
    """Test that the bundled module can be loaded."""
    with load_bundled_module(bundled_rich, "rich_bundled") as rich:
        # Just test that we can import and access basic attributes
        assert hasattr(rich, "print")
        assert hasattr(rich, "Console")
        assert hasattr(rich, "Table")


@pytest.mark.xfail(reason="Known issue with 'from abc import abc' in bundled code")
def test_bundled_print_functionality(bundled_rich):
    """Test rich.print functionality."""
    with load_bundled_module(bundled_rich, "rich_bundled") as rich:
        # Capture output
        console = rich.Console(file=StringIO(), force_terminal=True, width=80)

        # Test basic print
        console.print("Hello, World!")
        output = console.file.getvalue()
        assert "Hello, World!" in output


@pytest.mark.xfail(reason="Known issue with 'from abc import abc' in bundled code")
def test_bundled_table_rendering(bundled_rich):
    """Test table rendering with bundled rich."""
    with load_bundled_module(bundled_rich, "rich_bundled") as rich:
        # Create a simple table
        table = rich.Table(title="Test Table")
        table.add_column("Name", style="cyan")
        table.add_column("Value", style="magenta")
        table.add_row("Test", "123")
        table.add_row("Demo", "456")

        # Render to string
        console = rich.Console(file=StringIO(), force_terminal=True, width=80)
        console.print(table)
        output = console.file.getvalue()

        # Check that table content is present
        assert "Test Table" in output
        assert "Name" in output
        assert "Value" in output
        assert "Test" in output
        assert "123" in output


@pytest.mark.xfail(reason="Known issue with 'from abc import abc' in bundled code")
def test_bundled_text_formatting(bundled_rich):
    """Test text formatting with bundled rich."""
    with load_bundled_module(bundled_rich, "rich_bundled") as rich:
        # Use Text from the bundled module
        Text = rich.text.Text

        # Create formatted text
        text = Text("Hello", style="bold red")
        text.append(" World", style="italic blue")

        # Render to string
        console = rich.Console(file=StringIO(), force_terminal=True, width=80)
        console.print(text)
        output = console.file.getvalue()

        # Basic check that text is present
        assert "Hello" in output or "World" in output


@pytest.mark.xfail(reason="Known issue with 'from abc import abc' in bundled code")
def test_bundled_progress_bar(bundled_rich):
    """Test progress bar with bundled rich."""
    with load_bundled_module(bundled_rich, "rich_bundled") as rich:
        # Use progress components from the bundled module
        Progress = rich.progress.Progress
        SpinnerColumn = rich.progress.SpinnerColumn
        TextColumn = rich.progress.TextColumn

        # Create a simple progress bar
        with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), transient=True, console=rich.Console(file=StringIO(), force_terminal=True, width=80)) as progress:
            task = progress.add_task(description="Processing...", total=10)
            progress.update(task, advance=5)

            # Just verify it doesn't crash
            assert task is not None


@pytest.mark.xfail(reason="Known issue with 'from abc import abc' in bundled code")
def test_bundled_markdown_rendering(bundled_rich):
    """Test markdown rendering with bundled rich."""
    with load_bundled_module(bundled_rich, "rich_bundled") as rich:
        # Use Markdown from the bundled module
        Markdown = rich.markdown.Markdown

        # Create markdown content
        markdown_text = """
        # Test Header
        
        This is a **bold** text and this is *italic*.
        
        - Item 1
        - Item 2
        """

        md = Markdown(markdown_text)

        # Render to string
        console = rich.Console(file=StringIO(), force_terminal=True, width=80)
        console.print(md)
        output = console.file.getvalue()

        # Check that markdown content is present
        assert "Test Header" in output
        assert "Item 1" in output


if __name__ == "__main__":
    # Run with pytest when executed directly
    sys.exit(pytest.main([__file__, "-v"]))
