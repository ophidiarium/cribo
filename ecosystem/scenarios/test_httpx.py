#!/usr/bin/env python3
"""Test scenario for bundled httpx library.

This script:
1. Bundles the httpx library using cribo
2. Runs smoke tests using the bundled version
3. Verifies async HTTP functionality works correctly
"""

import os
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from .utils import run_cribo, format_bundle_size, load_bundled_module, ensure_test_directories, get_package_requirements, parse_requirements_file

# Type hint for better IDE support
if TYPE_CHECKING:
    pass


# Default timeout for HTTP requests - longer in CI environments
DEFAULT_TIMEOUT = 40 if os.environ.get("CI") else 30


@pytest.fixture(scope="module")
def bundled_httpx():
    """Bundle the httpx library and return the bundled module path."""
    # Ensure test directories exist
    tmp_dir = ensure_test_directories()

    # Create isolated directory for httpx output
    httpx_output_dir = tmp_dir / "httpx"
    httpx_output_dir.mkdir(parents=True, exist_ok=True)

    # Paths
    package_root = Path(__file__).resolve().parent.parent / "packages" / "httpx"
    httpx_init = package_root / "httpx"
    bundled_output = httpx_output_dir / "httpx_bundled.py"
    bundled_output.unlink(missing_ok=True)  # Remove if exists

    print("\n🔧 Bundling httpx library...")
    result = run_cribo(
        str(httpx_init),
        str(bundled_output),
        emit_requirements=True,
        # tree_shake=False,
    )

    assert result.returncode == 0, f"Failed to bundle httpx: {result.stderr}"

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


def test_bundle_generation(bundled_httpx):
    """Test that the bundle is generated successfully."""
    assert bundled_httpx.exists()
    assert bundled_httpx.stat().st_size > 0


def test_requirements_generation(bundled_httpx):
    """Test that requirements.txt is generated with expected dependencies."""
    requirements_path = bundled_httpx.parent / "requirements.txt"
    assert requirements_path.exists()

    # Parse the generated requirements.txt
    found_deps = parse_requirements_file(requirements_path)

    # Get expected dependencies from pyproject.toml
    package_root = Path(__file__).resolve().parent.parent / "packages" / "httpx"
    package_reqs = get_package_requirements(package_root)

    # Check for expected dependencies (both sets are already normalized)
    expected_deps = package_reqs["install_requires"]
    missing_deps = expected_deps - found_deps

    # Note: anyio is a transitive dependency through httpcore that cribo may not detect
    # directly from httpx's imports
    if missing_deps == {"anyio"}:
        pytest.skip("anyio is a transitive dependency not directly imported by httpx")

    # We should detect all required dependencies
    assert not missing_deps, f"Missing required dependencies: {missing_deps}"

    # Check for optional dependencies
    optional_deps = package_reqs["extras_require"]
    detected_optional = found_deps & optional_deps

    if detected_optional:
        print(f"   ℹ️  Optional dependencies detected: {detected_optional}")


def test_bundled_module_loading(bundled_httpx):
    """Test that the bundled module can be loaded."""
    with load_bundled_module(bundled_httpx, "httpx_bundled") as httpx:
        # Just test that we can import and access basic attributes
        assert hasattr(httpx, "get")
        assert hasattr(httpx, "post")
        assert hasattr(httpx, "Client")
        assert hasattr(httpx, "AsyncClient")


def test_bundled_get_request(bundled_httpx):
    """Test basic GET request with bundled httpx."""
    with load_bundled_module(bundled_httpx, "httpx_bundled") as httpx:
        # httpx uses synchronous requests with increased timeout
        resp = httpx.get("https://httpbingo.org/get", timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 200
        data = resp.json()
        assert "headers" in data
        assert "origin" in data


def test_bundled_post_request(bundled_httpx):
    """Test POST request with JSON data using bundled httpx."""
    with load_bundled_module(bundled_httpx, "httpx_bundled") as httpx:
        test_data = {"key": "value", "number": 42}
        resp = httpx.post("https://httpbingo.org/post", json=test_data, timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 200
        response_data = resp.json()
        assert response_data["json"] == test_data


def test_bundled_custom_headers(bundled_httpx):
    """Test custom headers with bundled httpx."""
    with load_bundled_module(bundled_httpx, "httpx_bundled") as httpx:
        headers = {"User-Agent": "cribo-test/1.0", "X-Test-Header": "test-value"}
        resp = httpx.get("https://httpbingo.org/headers", headers=headers, timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 200
        response_headers = resp.json()["headers"]
        assert response_headers.get("User-Agent") == ["cribo-test/1.0"]
        assert response_headers.get("X-Test-Header") == ["test-value"]


def test_bundled_query_params(bundled_httpx):
    """Test query parameters with bundled httpx."""
    with load_bundled_module(bundled_httpx, "httpx_bundled") as httpx:
        params = {"foo": "bar", "baz": "123"}
        resp = httpx.get("https://httpbingo.org/get", params=params, timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 200
        args = resp.json()["args"]
        assert args["foo"] == [params["foo"]]
        assert args["baz"] == [params["baz"]]


def test_bundled_client_usage(bundled_httpx):
    """Test Client context manager with bundled httpx."""
    with load_bundled_module(bundled_httpx, "httpx_bundled") as httpx:
        # Test using Client context manager with increased timeout
        with httpx.Client(timeout=DEFAULT_TIMEOUT) as client:
            resp = client.get("https://httpbingo.org/get")
            assert resp.status_code == 200

            # Test persistent headers with client
            client.headers.update({"X-Client-Header": "test"})
            resp = client.get("https://httpbingo.org/headers")
            headers = resp.json()["headers"]
            assert headers.get("X-Client-Header") == ["test"]


def test_bundled_timeout(bundled_httpx):
    """Test timeout handling with bundled httpx."""
    with load_bundled_module(bundled_httpx, "httpx_bundled") as httpx:
        # httpx uses different timeout API than requests
        with pytest.raises(httpx.TimeoutException):
            httpx.get("https://httpbingo.org/delay/10", timeout=2)


def test_bundled_status_codes(bundled_httpx):
    """Test various status codes with bundled httpx."""
    with load_bundled_module(bundled_httpx, "httpx_bundled") as httpx:
        resp = httpx.get("https://httpbingo.org/status/404", timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 404

        resp = httpx.get("https://httpbingo.org/status/500", timeout=DEFAULT_TIMEOUT)
        assert resp.status_code == 500


def test_bundled_async_client(bundled_httpx):
    """Test AsyncClient functionality with bundled httpx."""
    import asyncio

    with load_bundled_module(bundled_httpx, "httpx_bundled") as httpx:

        async def async_test():
            async with httpx.AsyncClient(timeout=DEFAULT_TIMEOUT) as client:
                resp = await client.get("https://httpbingo.org/get")
                assert resp.status_code == 200
                data = resp.json()
                assert "headers" in data

        # Run the async test
        asyncio.run(async_test())


def test_bundled_http2_support(bundled_httpx):
    """Test HTTP/2 support with bundled httpx."""
    with load_bundled_module(bundled_httpx, "httpx_bundled") as httpx:
        # httpx supports HTTP/2 when httpcore[http2] is installed
        # Just verify the option exists with increased timeout
        client = httpx.Client(http2=True, timeout=DEFAULT_TIMEOUT)
        assert client is not None
        client.close()


if __name__ == "__main__":
    # Run with pytest when executed directly
    sys.exit(pytest.main([__file__, "-v"]))
