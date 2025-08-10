#!/usr/bin/env python3
"""Test scenario for bundled rich library.

This script:
1. Bundles the rich library using cribo
2. Runs smoke tests using the bundled version
3. Tests various rich formatting and display features

Rich is a Python library for rich text and beautiful formatting in the terminal.
It provides advanced features like:
- Colored text and syntax highlighting
- Tables, progress bars, and panels
- Markdown rendering
- Tracebacks with syntax highlighting
"""

import sys
import io
from pathlib import Path
from types import ModuleType
from typing import TYPE_CHECKING

from .utils import run_cribo, format_bundle_size, load_bundled_module, ensure_test_directories, get_package_requirements

# Type hint for better IDE support
if TYPE_CHECKING:
    import rich as RichType


def run_smoke_tests(rich: "ModuleType | RichType"):
    """Run smoke tests using the bundled rich module.

    Args:
        rich: The dynamically imported bundled rich module
    """
    print("🧪 Running smoke tests...")

    # Test 1: Basic console output with colors
    print("  1. Testing console with colors...")
    from io import StringIO

    console_output = StringIO()
    console = rich.console.Console(file=console_output, force_terminal=True, width=80)
    console.print("[bold red]Hello[/bold red] [green]World[/green]!")
    output = console_output.getvalue()
    assert "Hello" in output and "World" in output
    print("     ✓ Console output successful")

    # Test 2: Creating and rendering a table
    print("  2. Testing table rendering...")
    table = rich.table.Table(title="Test Table")
    table.add_column("Name", style="cyan", no_wrap=True)
    table.add_column("Age", style="magenta")
    table.add_column("City", style="green")

    table.add_row("Alice", "30", "New York")
    table.add_row("Bob", "25", "San Francisco")
    table.add_row("Charlie", "35", "Chicago")

    console_output = StringIO()
    console = rich.console.Console(file=console_output, force_terminal=True, width=80)
    console.print(table)
    output = console_output.getvalue()
    assert "Test Table" in output
    assert "Alice" in output and "Bob" in output and "Charlie" in output
    print("     ✓ Table rendering successful")

    # Test 3: Panel with borders
    print("  3. Testing panel rendering...")
    panel = rich.panel.Panel("[bold]Important Message[/bold]\nThis is a test panel.", title="Notice", border_style="blue")
    console_output = StringIO()
    console = rich.console.Console(file=console_output, force_terminal=True, width=80)
    console.print(panel)
    output = console_output.getvalue()
    assert "Notice" in output
    assert "Important Message" in output
    print("     ✓ Panel rendering successful")

    # Test 4: Progress bar
    print("  4. Testing progress bar...")
    console_output = StringIO()
    console = rich.console.Console(file=console_output, force_terminal=True, width=80)

    # Create a simple progress bar
    progress = rich.progress.Progress(rich.progress.TextColumn("[progress.description]{task.description}"), rich.progress.BarColumn(), rich.progress.TaskProgressColumn(), console=console, transient=True)

    with progress:
        task = progress.add_task("Processing...", total=100)
        progress.update(task, advance=50)
        progress.update(task, advance=50)

    # Progress bars might not produce output in non-TTY mode, just verify no errors
    print("     ✓ Progress bar creation successful")

    # Test 5: Syntax highlighting
    print("  5. Testing syntax highlighting...")
    code = '''def hello(name):
    """Say hello to someone."""
    print(f"Hello, {name}!")
    return True'''

    syntax = rich.syntax.Syntax(code, "python", theme="monokai", line_numbers=True)
    console_output = StringIO()
    console = rich.console.Console(file=console_output, force_terminal=True, width=80)
    console.print(syntax)
    output = console_output.getvalue()
    assert "hello" in output
    assert "name" in output
    print("     ✓ Syntax highlighting successful")

    # Test 6: Markdown rendering
    print("  6. Testing markdown rendering...")
    markdown_text = """# Header 1

This is **bold** text and this is *italic* text.

- Item 1
- Item 2
- Item 3

```python
print("Hello from code block")
```
"""

    markdown = rich.markdown.Markdown(markdown_text)
    console_output = StringIO()
    console = rich.console.Console(file=console_output, force_terminal=True, width=80)
    console.print(markdown)
    output = console_output.getvalue()
    assert "Header 1" in output
    assert "bold" in output
    assert "Item 1" in output
    print("     ✓ Markdown rendering successful")

    # Test 7: Text styling and alignment
    print("  7. Testing text styling...")
    text = rich.text.Text("Styled Text Example")
    text.stylize("bold magenta", 0, 6)
    text.stylize("italic cyan", 7, 11)
    text.stylize("underline yellow", 12, 19)

    console_output = StringIO()
    console = rich.console.Console(file=console_output, force_terminal=True, width=80)
    console.print(text)
    output = console_output.getvalue()
    assert "Styled" in output and "Text" in output and "Example" in output
    print("     ✓ Text styling successful")

    # Test 8: Tree structure
    print("  8. Testing tree rendering...")
    tree = rich.tree.Tree("Root")
    branch1 = tree.add("Branch 1")
    branch1.add("Leaf 1.1")
    branch1.add("Leaf 1.2")
    branch2 = tree.add("Branch 2")
    branch2.add("Leaf 2.1")

    console_output = StringIO()
    console = rich.console.Console(file=console_output, force_terminal=True, width=80)
    console.print(tree)
    output = console_output.getvalue()
    assert "Root" in output
    assert "Branch 1" in output
    assert "Leaf 1.1" in output
    print("     ✓ Tree rendering successful")

    # Test 9: Pretty printing data structures
    print("  9. Testing pretty printing...")
    data = {"users": [{"name": "Alice", "age": 30, "active": True}, {"name": "Bob", "age": 25, "active": False}], "settings": {"theme": "dark", "notifications": True}}

    console_output = StringIO()
    console = rich.console.Console(file=console_output, force_terminal=True, width=80)
    console.print(data)
    output = console_output.getvalue()
    assert "Alice" in output
    assert "theme" in output
    print("     ✓ Pretty printing successful")

    # Test 10: Rule/separator
    print("  10. Testing rule rendering...")
    console_output = StringIO()
    console = rich.console.Console(file=console_output, force_terminal=True, width=80)
    console.rule("[bold red]Section Divider[/bold red]")
    output = console_output.getvalue()
    assert "Section Divider" in output
    print("     ✓ Rule rendering successful")

    print("\n✅ All smoke tests passed!")


def test_rich_bundled():
    """Test the bundled rich library."""
    # Ensure test directories exist
    tmp_dir = ensure_test_directories()

    # Paths
    rich_init = Path(__file__).parent.parent / "packages" / "rich" / "rich"
    bundled_output = tmp_dir / "rich_bundled.py"

    print("🔧 Bundling rich library...")
    result = run_cribo(
        str(rich_init),
        str(bundled_output),
        emit_requirements=True,
        # tree_shake=False,  # Rich uses dynamic imports extensively
    )

    if result.returncode != 0:
        sys.exit(1)

    bundle_size = bundled_output.stat().st_size
    print(f"✅ Successfully bundled to {bundled_output}")
    print(f"   Bundle size: {format_bundle_size(bundle_size)}")

    # Check and verify requirements.txt was generated correctly
    print("\n📋 Checking generated requirements.txt...")
    requirements_path = bundled_output.parent / "requirements.txt"

    if not requirements_path.exists():
        print("❌ requirements.txt was not generated!")
        sys.exit(1)

    requirements_content = requirements_path.read_text().strip()
    print(f"   Generated at: {requirements_path}")
    print("   Content:")
    for line in requirements_content.splitlines():
        print(f"     - {line}")

    # Get expected dependencies from setup.py or pyproject.toml
    package_root = Path(__file__).resolve().parent.parent / "packages" / "rich"
    package_reqs = get_package_requirements(package_root)

    print("\n   Expected dependencies from package metadata:")
    for dep in sorted(package_reqs["install_requires"]):
        print(f"     - {dep}")

    if package_reqs["extras_require"]:
        print("\n   Optional dependencies from extras_require:")
        for dep in sorted(package_reqs["extras_require"]):
            print(f"     - {dep}")

    # Verify expected dependencies for rich
    # Note: cribo detects all possible imports, not just runtime dependencies
    expected_deps = package_reqs["install_requires"]

    # Optional/conditional dependencies that may be detected
    # Include both extras_require and known conditional imports
    optional_deps = package_reqs["extras_require"]

    # Parse the requirements
    found_deps = set()
    for line in requirements_content.splitlines():
        if line and not line.startswith("#"):
            # Extract package name (before any version specifier)
            pkg_name = line.split(">=")[0].split("==")[0].split("<")[0].split(">")[0].strip()
            found_deps.add(pkg_name)

    missing_deps = expected_deps - found_deps
    unexpected_deps = found_deps - expected_deps - optional_deps
    detected_optional = found_deps & optional_deps

    if missing_deps:
        print(f"   ❌ Missing expected dependencies: {missing_deps}")
        sys.exit(1)

    if detected_optional:
        print(f"   ℹ️  Optional dependencies detected: {detected_optional}")

    if unexpected_deps:
        print(f"   ⚠️  Unexpected dependencies found: {unexpected_deps}")
        # Don't fail on unexpected deps, just warn

    print("   ✓ All required dependencies found")

    # Run smoke tests by importing the bundled module
    print("\n🧪 Running smoke tests with bundled library...")

    try:
        with load_bundled_module(bundled_output, "rich_bundled") as rich:
            run_smoke_tests(rich)
    except Exception as e:
        print(f"❌ Failed to load or test bundled module: {e}")
        import traceback

        traceback.print_exc()
        sys.exit(1)

    return True


def main():
    """Main entry point."""
    print("🚀 Ecosystem test: rich")
    print("=" * 50)

    try:
        test_rich_bundled()
        print("\n✅ Ecosystem test completed successfully!")
        return 0
    except Exception as e:
        print(f"\n❌ Ecosystem test failed: {e}")
        import traceback

        traceback.print_exc()
        return 1


if __name__ == "__main__":
    sys.exit(main())
