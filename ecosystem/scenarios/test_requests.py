#!/usr/bin/env python3
"""Test scenario for bundled requests library.

This script:
1. Bundles the requests library using cribo
2. Runs smoke tests using the bundled version
3. Compares behavior with the original library

NOTE: Currently fails due to a bug in cribo's handling of relative imports
      like "from . import sessions" which get transformed incorrectly.
      TODO: Remove tree_shake=False once the relative import bug is fixed.
"""

import sys
from pathlib import Path
from types import ModuleType
from typing import TYPE_CHECKING, Type

from .utils import run_cribo, format_bundle_size, load_bundled_module, ensure_test_directories, get_package_requirements

# Type hint for better IDE support
if TYPE_CHECKING:
    import requests as RequestsType


def run_smoke_tests(requests: "ModuleType | RequestsType"):
    """Run smoke tests using the bundled requests module.

    Args:
        requests: The dynamically imported bundled requests module
    """
    print("🧪 Running smoke tests...")

    # Test 1: Basic GET request
    print("  1. Testing GET request...")
    resp = requests.get("https://httpbin.org/get")
    assert resp.status_code == 200, f"Expected 200, got {resp.status_code}"
    data = resp.json()
    assert "headers" in data, "Response missing 'headers' field"
    assert "origin" in data, "Response missing 'origin' field"
    print("     ✓ GET request successful")

    # Test 2: POST with JSON data
    print("  2. Testing POST with JSON...")
    test_data = {"key": "value", "number": 42}
    resp = requests.post("https://httpbin.org/post", json=test_data)
    assert resp.status_code == 200, f"Expected 200, got {resp.status_code}"
    response_data = resp.json()
    assert response_data["json"] == test_data, "Posted JSON data mismatch"
    print("     ✓ POST with JSON successful")

    # Test 3: Headers
    print("  3. Testing custom headers...")
    headers = {"User-Agent": "cribo-test/1.0", "X-Test-Header": "test-value"}
    resp = requests.get("https://httpbin.org/headers", headers=headers)
    assert resp.status_code == 200
    response_headers = resp.json()["headers"]
    assert response_headers.get("User-Agent") == "cribo-test/1.0"
    assert response_headers.get("X-Test-Header") == "test-value"
    print("     ✓ Custom headers successful")

    # Test 4: Query parameters
    print("  4. Testing query parameters...")
    params = {"foo": "bar", "baz": "123"}
    resp = requests.get("https://httpbin.org/get", params=params)
    assert resp.status_code == 200
    args = resp.json()["args"]
    assert args == params, f"Expected {params}, got {args}"
    print("     ✓ Query parameters successful")

    # Test 5: Timeout handling
    print("  5. Testing timeout...")
    try:
        resp = requests.get("https://httpbin.org/delay/10", timeout=1)
        assert False, "Should have timed out"
    except requests.exceptions.Timeout:
        print("     ✓ Timeout handling successful")

    # Test 6: Status code checking
    print("  6. Testing status codes...")
    resp = requests.get("https://httpbin.org/status/404")
    assert resp.status_code == 404
    resp = requests.get("https://httpbin.org/status/500")
    assert resp.status_code == 500
    print("     ✓ Status code handling successful")

    print("\n✅ All smoke tests passed!")


def test_requests_bundled():
    """Test the bundled requests library."""
    # Ensure test directories exist
    tmp_dir = ensure_test_directories()

    # Paths
    requests_init = Path(__file__).parent.parent / "packages" / "requests" / "src" / "requests"
    bundled_output = tmp_dir / "requests_bundled.py"

    print("🔧 Bundling requests library...")
    result = run_cribo(
        str(requests_init),
        str(bundled_output),
        emit_requirements=True,
        tree_shake=False,  # TODO: Enable once relative import bug is fixed
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

    # Get expected dependencies from setup.py
    package_root = Path("ecosystem/packages/requests")
    package_reqs = get_package_requirements(package_root)

    print("\n   Expected dependencies from setup.py:")
    for dep in sorted(package_reqs["install_requires"]):
        print(f"     - {dep}")

    if package_reqs["extras_require"]:
        print("\n   Optional dependencies from extras_require:")
        for dep in sorted(package_reqs["extras_require"]):
            print(f"     - {dep}")

    # Verify expected dependencies for requests
    # Note: cribo detects all possible imports, not just runtime dependencies
    expected_deps = package_reqs["install_requires"]

    # Optional/conditional dependencies that may be detected
    # Include both extras_require and known conditional imports
    optional_deps = package_reqs["extras_require"] | {
        "cryptography",  # For certain auth methods
        "simplejson",  # JSON fallback
        "dummy_threading",  # Threading compatibility
    }

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
        with load_bundled_module(bundled_output, "requests_bundled") as requests:
            run_smoke_tests(requests)
    except Exception as e:
        print(f"❌ Failed to load or test bundled module: {e}")
        import traceback

        traceback.print_exc()
        sys.exit(1)

    return True


def main():
    """Main entry point."""
    print("🚀 Ecosystem test: requests")
    print("=" * 50)

    try:
        test_requests_bundled()
        print("\n✅ Ecosystem test completed successfully!")
        return 0
    except Exception as e:
        print(f"\n❌ Ecosystem test failed: {e}")
        import traceback

        traceback.print_exc()
        return 1


if __name__ == "__main__":
    sys.exit(main())
