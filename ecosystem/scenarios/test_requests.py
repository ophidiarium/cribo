#!/usr/bin/env python3
"""Test scenario for bundled requests library.

This script:
1. Bundles the requests library using cribo
2. Runs smoke tests using the bundled version
3. Compares behavior with the original library
"""

import sys
from pathlib import Path
from types import ModuleType
from typing import TYPE_CHECKING

import pytest

from .utils import run_cribo, format_bundle_size, load_bundled_module, ensure_test_directories, get_package_requirements

# Type hint for better IDE support
if TYPE_CHECKING:
    import requests as RequestsType


@pytest.fixture(scope="module")
def bundled_requests():
    """Bundle the requests library and return the bundled module."""
    # Ensure test directories exist
    tmp_dir = ensure_test_directories()

    # Paths
    requests_init = Path(__file__).parent.parent / "packages" / "requests" / "src" / "requests"
    bundled_output = tmp_dir / "requests_bundled.py"

    print("\n🔧 Bundling requests library...")
    result = run_cribo(
        str(requests_init),
        str(bundled_output),
        emit_requirements=True,
        tree_shake=False,  # TODO: Enable once relative import bug is fixed
    )

    if result.returncode != 0:
        pytest.fail(f"Failed to bundle requests: {result.stderr}")

    bundle_size = bundled_output.stat().st_size
    print(f"✅ Successfully bundled to {bundled_output}")
    print(f"   Bundle size: {format_bundle_size(bundle_size)}")

    # Check requirements.txt generation
    requirements_path = bundled_output.parent / "requirements.txt"
    if not requirements_path.exists():
        pytest.fail("requirements.txt was not generated!")

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

    requirements_content = requirements_path.read_text().strip()

    # Get expected dependencies from setup.py
    package_root = Path(__file__).parent.parent / "packages" / "requests"
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


@pytest.mark.xfail(reason="Known issue with namespace handling in bundled code")
def test_bundled_module_loading(bundled_requests):
    """Test that the bundled module can be loaded."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        # Just test that we can import and access basic attributes
        assert hasattr(requests, "get")
        assert hasattr(requests, "post")
        assert hasattr(requests, "Session")


@pytest.mark.xfail(reason="Known issue with namespace handling in bundled code")
def test_bundled_get_request(bundled_requests):
    """Test basic GET request with bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        resp = requests.get("https://httpbin.org/get")
        assert resp.status_code == 200
        data = resp.json()
        assert "headers" in data
        assert "origin" in data


@pytest.mark.xfail(reason="Known issue with namespace handling in bundled code")
def test_bundled_post_request(bundled_requests):
    """Test POST request with JSON data using bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        test_data = {"key": "value", "number": 42}
        resp = requests.post("https://httpbin.org/post", json=test_data)
        assert resp.status_code == 200
        response_data = resp.json()
        assert response_data["json"] == test_data


@pytest.mark.xfail(reason="Known issue with namespace handling in bundled code")
def test_bundled_custom_headers(bundled_requests):
    """Test custom headers with bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        headers = {"User-Agent": "cribo-test/1.0", "X-Test-Header": "test-value"}
        resp = requests.get("https://httpbin.org/headers", headers=headers)
        assert resp.status_code == 200
        response_headers = resp.json()["headers"]
        assert response_headers.get("User-Agent") == "cribo-test/1.0"
        assert response_headers.get("X-Test-Header") == "test-value"


@pytest.mark.xfail(reason="Known issue with namespace handling in bundled code")
def test_bundled_query_params(bundled_requests):
    """Test query parameters with bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        params = {"foo": "bar", "baz": "123"}
        resp = requests.get("https://httpbin.org/get", params=params)
        assert resp.status_code == 200
        args = resp.json()["args"]
        assert args == params


@pytest.mark.xfail(reason="Known issue with namespace handling in bundled code")
def test_bundled_timeout(bundled_requests):
    """Test timeout handling with bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        with pytest.raises(requests.exceptions.Timeout):
            requests.get("https://httpbin.org/delay/10", timeout=1)


@pytest.mark.xfail(reason="Known issue with namespace handling in bundled code")
def test_bundled_status_codes(bundled_requests):
    """Test various status codes with bundled requests."""
    with load_bundled_module(bundled_requests, "requests_bundled") as requests:
        resp = requests.get("https://httpbin.org/status/404")
        assert resp.status_code == 404

        resp = requests.get("https://httpbin.org/status/500")
        assert resp.status_code == 500


if __name__ == "__main__":
    # Run with pytest when executed directly
    sys.exit(pytest.main([__file__, "-v"]))
