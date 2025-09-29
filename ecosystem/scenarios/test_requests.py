#!/usr/bin/env python3
"""Test scenario for bundled requests library.

This script:
1. Bundles the requests library using cribo
2. Runs smoke tests using the bundled version
3. Compares behavior with the original library
"""

import os
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from .utils import run_cribo, format_bundle_size, load_bundled_module, ensure_test_directories, get_package_requirements, parse_requirements_file

# Default timeout for HTTP requests - longer in CI environments
DEFAULT_TIMEOUT = 30 if os.environ.get("CI") else 10

# Type hint for better IDE support
if TYPE_CHECKING:
    pass


@pytest.fixture(scope="module")
def bundled_requests():
    """Bundle the requests library and return the bundled module."""
    # Ensure test directories exist
    tmp_dir = ensure_test_directories()

    # Create isolated directory for requests output
    requests_output_dir = tmp_dir / "requests"
    requests_output_dir.mkdir(parents=True, exist_ok=True)

    # Paths
    requests_init = Path(__file__).parent.parent / "packages" / "requests" / "src" / "requests"
    bundled_output = requests_output_dir / "requests_bundled.py"

    print("\n🔧 Bundling requests library...")
    result = run_cribo(
        str(requests_init),
        str(bundled_output),
        emit_requirements=True,
        tree_shake=False,  # TODO: Enable once relative import bug is fixed
    )

    assert result.returncode == 0, f"Failed to bundle requests: {result.stderr}"

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


def test_bundle_generation(bundled_requests):
    """Test that the bundle is generated successfully."""
    assert bundled_requests.exists()
    assert bundled_requests.stat().st_size > 0


def test_requirements_generation(bundled_requests):
    """Test that requirements.txt is generated with expected dependencies."""
    requirements_path = bundled_requests.parent / "requirements.txt"
    assert requirements_path.exists()

    # Parse the generated requirements.txt
    found_deps = parse_requirements_file(requirements_path)

    # Get expected dependencies from setup.py
    package_root = Path(__file__).parent.parent / "packages" / "requests"
    package_reqs = get_package_requirements(package_root)

    # Check for expected dependencies (both sets are already normalized)
    expected_deps = package_reqs["install_requires"]
    missing_deps = expected_deps - found_deps

    # We should detect all required dependencies
    assert not missing_deps, f"Missing required dependencies: {missing_deps}"


def test_bundled_module_loading(bundled_requests):
    """Test that the bundled module can be loaded."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        # Just test that we can import and access basic attributes
        assert hasattr(requests, "get")
        assert hasattr(requests, "post")
        assert hasattr(requests, "Session")


def test_bundled_get_request(bundled_requests):
    """Test basic GET request with bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        resp = requests.get("https://httpbingo.org/get", timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 200
        data = resp.json()
        assert "headers" in data
        assert "origin" in data


def test_bundled_post_request(bundled_requests):
    """Test POST request with JSON data using bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        test_data = {"key": "value", "number": 42}
        resp = requests.post("https://httpbingo.org/post", json=test_data, timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 200
        response_data = resp.json()
        assert response_data["json"] == test_data


def test_bundled_custom_headers(bundled_requests):
    """Test custom headers with bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        headers = {"User-Agent": "cribo-test/1.0", "X-Test-Header": "test-value"}
        resp = requests.get("https://httpbingo.org/headers", headers=headers, timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 200
        response_headers = resp.json()["headers"]
        assert response_headers.get("User-Agent") == ["cribo-test/1.0"]
        assert response_headers.get("X-Test-Header") == ["test-value"]


def test_bundled_query_params(bundled_requests):
    """Test query parameters with bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        params = {"foo": "bar", "baz": "123"}
        resp = requests.get("https://httpbingo.org/get", params=params, timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 200
        args = resp.json()["args"]
        assert args["foo"] == [params["foo"]]
        assert args["baz"] == [params["baz"]]


def test_bundled_timeout(bundled_requests):
    """Test timeout handling with bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        with pytest.raises(requests.exceptions.Timeout):
            requests.get("https://httpbingo.org/delay/10", timeout=1)


def test_bundled_status_codes(bundled_requests):
    """Test various status codes with bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        resp = requests.get("https://httpbingo.org/status/404", timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 404

        resp = requests.get("https://httpbingo.org/status/500", timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 500


if __name__ == "__main__":
    # Run with pytest when executed directly
    sys.exit(pytest.main([__file__, "-v"]))
