#!/usr/bin/env python3
"""Test scenario for bundled pyyaml library.

This script:
1. Bundles the pyyaml library using cribo
2. Runs smoke tests using the bundled version
3. Verifies YAML parsing and dumping functionality works correctly
"""

import sys
from pathlib import Path
from io import StringIO
from types import ModuleType
from typing import TYPE_CHECKING

import pytest

from .utils import run_cribo, format_bundle_size, load_bundled_module, ensure_test_directories, get_package_requirements, parse_requirements_file

# Type hint for better IDE support
if TYPE_CHECKING:
    import yaml as YamlType


@pytest.fixture(scope="module")
def bundled_pyyaml():
    """Bundle the pyyaml library and return the bundled module path."""
    # Ensure test directories exist
    tmp_dir = ensure_test_directories()

    # Paths
    package_root = Path(__file__).resolve().parent.parent / "packages" / "pyyaml"
    yaml_init = package_root / "lib" / "yaml"
    bundled_output = tmp_dir / "pyyaml_bundled.py"

    print("\n🔧 Bundling pyyaml library...")
    print("   Note: PyYAML has an optional C extension (_yaml) that will be ignored")
    result = run_cribo(
        str(yaml_init),
        str(bundled_output),
        emit_requirements=True,
        # PyYAML is a pure Python library, tree-shaking should work
    )

    # PyYAML has optional C extensions that cribo now handles correctly:
    # - yaml._yaml is detected as a NativeExtension and left as an import
    # - The bundled code will work with the pure Python fallback

    assert result.returncode == 0, f"Failed to bundle pyyaml: {result.stderr}"

    bundle_size = bundled_output.stat().st_size
    print(f"✅ Successfully bundled to {bundled_output}")
    print(f"   Bundle size: {format_bundle_size(bundle_size)}")

    # Check requirements.txt generation
    requirements_path = bundled_output.parent / "requirements.txt"
    assert requirements_path.exists(), "requirements.txt was not generated!"

    requirements_content = requirements_path.read_text().strip()
    print(f"\n📋 Generated requirements.txt at: {requirements_path}")
    print("   Content:")
    if requirements_content:
        for line in requirements_content.splitlines():
            print(f"     - {line}")
    else:
        print("     (empty - no external dependencies)")

    # Return path for loading
    return bundled_output


def test_bundle_generation(bundled_pyyaml):
    """Test that the bundle is generated successfully."""
    assert bundled_pyyaml.exists()
    assert bundled_pyyaml.stat().st_size > 0


def test_requirements_generation(bundled_pyyaml):
    """Test that requirements.txt is generated with expected dependencies."""
    requirements_path = bundled_pyyaml.parent / "requirements.txt"
    assert requirements_path.exists()

    # Parse the generated requirements.txt
    found_deps = parse_requirements_file(requirements_path)

    # Get expected dependencies from setup.py
    package_root = Path(__file__).resolve().parent.parent / "packages" / "pyyaml"
    package_reqs = get_package_requirements(package_root)

    # Check for expected dependencies (both sets are already normalized)
    expected_deps = package_reqs["install_requires"]
    missing_deps = expected_deps - found_deps

    # PyYAML is a pure Python library with no required dependencies
    # The C extension (_yaml) is optional
    assert not missing_deps, f"Missing required dependencies: {missing_deps}"


def test_bundled_module_loading(bundled_pyyaml):
    """Test that the bundled module can be loaded."""
    with load_bundled_module(bundled_pyyaml, "pyyaml_bundled") as yaml:
        # Just test that we can import and access basic attributes
        assert hasattr(yaml, "load")
        assert hasattr(yaml, "dump")
        assert hasattr(yaml, "SafeLoader")
        assert hasattr(yaml, "SafeDumper")


def test_bundled_yaml_parsing(bundled_pyyaml):
    """Test YAML parsing with bundled pyyaml."""
    with load_bundled_module(bundled_pyyaml, "pyyaml_bundled") as yaml:
        # Test parsing a simple YAML string
        yaml_content = """
        name: Test Document
        version: 1.0
        features:
          - parsing
          - dumping
          - safe_loading
        config:
          debug: true
          timeout: 30
        """

        data = yaml.safe_load(yaml_content)

        assert data["name"] == "Test Document"
        assert data["version"] == 1.0
        assert "parsing" in data["features"]
        assert data["config"]["debug"] is True
        assert data["config"]["timeout"] == 30


def test_bundled_yaml_dumping(bundled_pyyaml):
    """Test YAML dumping with bundled pyyaml."""
    with load_bundled_module(bundled_pyyaml, "pyyaml_bundled") as yaml:
        # Test dumping Python data to YAML
        data = {"name": "Generated Document", "items": ["apple", "banana", "cherry"], "metadata": {"created": "2024-01-01", "modified": "2024-01-02", "version": 2}, "active": True}

        yaml_output = yaml.dump(data, default_flow_style=False)

        # Verify the output contains expected content
        assert "name: Generated Document" in yaml_output
        assert "apple" in yaml_output
        assert "created:" in yaml_output
        assert "active: true" in yaml_output


def test_bundled_yaml_roundtrip(bundled_pyyaml):
    """Test YAML roundtrip (dump and load) with bundled pyyaml."""
    with load_bundled_module(bundled_pyyaml, "pyyaml_bundled") as yaml:
        # Create test data
        original_data = {"string": "hello world", "number": 42, "float": 3.14159, "boolean": True, "null_value": None, "list": [1, 2, 3, 4, 5], "nested": {"key1": "value1", "key2": ["a", "b", "c"], "key3": {"deep": "nested"}}}

        # Dump to YAML and load back
        yaml_str = yaml.dump(original_data, default_flow_style=False)
        loaded_data = yaml.safe_load(yaml_str)

        # Verify roundtrip preserves data
        assert loaded_data == original_data


def test_bundled_yaml_multiple_documents(bundled_pyyaml):
    """Test handling multiple YAML documents with bundled pyyaml."""
    with load_bundled_module(bundled_pyyaml, "pyyaml_bundled") as yaml:
        # YAML with multiple documents
        multi_doc_yaml = """
---
document: 1
type: first
---
document: 2
type: second
---
document: 3
type: third
"""

        # Load all documents
        documents = list(yaml.safe_load_all(multi_doc_yaml))

        assert len(documents) == 3
        assert documents[0]["document"] == 1
        assert documents[1]["type"] == "second"
        assert documents[2]["document"] == 3


def test_bundled_yaml_anchors_and_aliases(bundled_pyyaml):
    """Test YAML anchors and aliases with bundled pyyaml."""
    with load_bundled_module(bundled_pyyaml, "pyyaml_bundled") as yaml:
        # YAML with anchors and aliases
        yaml_content = """
defaults: &defaults
  adapter: postgres
  host: localhost
  port: 5432

development:
  <<: *defaults
  database: dev_db

production:
  <<: *defaults
  database: prod_db
  host: prod.example.com
"""

        data = yaml.safe_load(yaml_content)

        # Verify aliases were expanded correctly
        assert data["development"]["adapter"] == "postgres"
        assert data["development"]["host"] == "localhost"
        assert data["development"]["database"] == "dev_db"

        assert data["production"]["adapter"] == "postgres"
        assert data["production"]["host"] == "prod.example.com"  # Overridden
        assert data["production"]["database"] == "prod_db"


def test_bundled_yaml_custom_tags(bundled_pyyaml):
    """Test custom YAML tags with bundled pyyaml."""
    with load_bundled_module(bundled_pyyaml, "pyyaml_bundled") as yaml:
        # Test that we can at least access the constructor mechanism
        # assert hasattr(yaml, "YAMLObject")
        # assert hasattr(yaml, "Constructor")

        # Test basic tag handling
        yaml_with_tag = """
        !!python/tuple [1, 2, 3]
        """

        # Use Loader instead of SafeLoader for this test
        # Note: In production, avoid using unsafe loaders
        data = yaml.load(yaml_with_tag, Loader=yaml.Loader)
        assert isinstance(data, tuple)
        assert data == (1, 2, 3)


def test_bundled_yaml_flow_style(bundled_pyyaml):
    """Test different YAML flow styles with bundled pyyaml."""
    with load_bundled_module(bundled_pyyaml, "pyyaml_bundled") as yaml:
        # Test data
        data = {"inline_list": [1, 2, 3], "inline_dict": {"a": 1, "b": 2}}

        # Dump with flow style
        flow_output = yaml.dump(data, default_flow_style=True)
        assert "{" in flow_output  # Flow style uses braces
        assert "[" in flow_output  # Flow style uses brackets

        # Dump with block style
        block_output = yaml.dump(data, default_flow_style=False)
        assert "- " in block_output  # Block style uses dashes for lists

        # Both should parse back to the same data
        assert yaml.safe_load(flow_output) == data
        assert yaml.safe_load(block_output) == data


if __name__ == "__main__":
    # Run with pytest when executed directly
    sys.exit(pytest.main([__file__, "-v"]))
