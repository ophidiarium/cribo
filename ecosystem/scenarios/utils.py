"""Shared utilities for ecosystem test scenarios."""

import importlib.util
import sys
import subprocess
from pathlib import Path
from typing import List, Optional, Any, Set, Dict
from contextlib import contextmanager


def ensure_test_directories():
    """Ensure all necessary test directories exist.

    Creates:
    - target/tmp: For temporary bundled output files

    Returns:
        Path to the tmp directory
    """
    # Get project root (ecosystem/scenarios/utils.py -> ../..)
    project_root = Path(__file__).resolve().parent.parent.parent
    tmp_dir = project_root / "target" / "tmp"

    # Create directory if it doesn't exist
    tmp_dir.mkdir(parents=True, exist_ok=True)

    return tmp_dir


def run_cribo(entry_point: str, output_path: str, emit_requirements: bool = True, tree_shake: bool = False, verbose: bool = False) -> subprocess.CompletedProcess:
    """Run cribo to bundle a Python module.

    Args:
        entry_point: Path to the entry point Python file
        output_path: Path where the bundled output should be saved
        emit_requirements: Whether to generate requirements.txt (default: True)
        tree_shake: Whether to enable tree-shaking (default: False)
        verbose: Whether to show verbose output (default: False)

    Returns:
        CompletedProcess instance with the result of running cribo
    """
    # Find cribo executable relative to this file's location
    # ecosystem/scenarios/utils.py -> ../../target/release/cribo
    cribo_path = Path(__file__).resolve().parent.parent.parent / "target" / "release" / "cribo"

    # Fall back to PATH if the release binary doesn't exist (e.g., in CI)
    if not cribo_path.exists():
        cribo_cmd = "cribo"
        if verbose:
            print(f"  Using cribo from PATH (release binary not found at {cribo_path})")
    else:
        cribo_cmd = str(cribo_path)
        if verbose:
            print(f"  Using cribo from: {cribo_path}")

    cmd: List[str] = [
        cribo_cmd,
        "--entry",
        entry_point,
        "--output",
        output_path,
    ]

    if emit_requirements:
        cmd.append("--emit-requirements")

    if not tree_shake:
        cmd.append("--no-tree-shake")

    if verbose:
        cmd.append("-v")

    result = subprocess.run(cmd, capture_output=True, text=True)

    if result.returncode != 0:
        print(f"Cribo failed with exit code {result.returncode}")
        print(f"STDOUT:\n{result.stdout}")
        print(f"STDERR:\n{result.stderr}")

    return result


def run_bundled_test(bundled_path: str, test_script: str) -> subprocess.CompletedProcess:
    """Run a test script with the bundled module.

    Args:
        bundled_path: Path to the bundled Python file
        test_script: Python code to execute for testing

    Returns:
        CompletedProcess instance with the test result
    """
    original_sys_path = sys.path.copy()
    try:
        # Insert the bundle directory into sys.path
        sys.path.insert(0, bundled_path)

        result = subprocess.run([sys.executable, "-c", test_script], capture_output=True, text=True)

        if result.returncode != 0:
            print(f"❌ Tests failed with exit code {result.returncode}")
            print(f"STDOUT:\n{result.stdout}")
            print(f"STDERR:\n{result.stderr}")

        return result
    finally:
        # Restore the original sys.path
        sys.path = original_sys_path


def format_bundle_size(size_bytes: int) -> str:
    """Format bundle size in human-readable format.

    Args:
        size_bytes: Size in bytes

    Returns:
        Formatted string with size
    """
    if size_bytes < 1024:
        return f"{size_bytes} bytes"
    elif size_bytes < 1024 * 1024:
        return f"{size_bytes / 1024:.1f} KB"
    else:
        return f"{size_bytes / (1024 * 1024):.1f} MB"


@contextmanager
def load_bundled_module(bundle_path: Path, module_name: str):
    """Context manager to safely load and unload a bundled module.

    Args:
        bundle_path: Path to the bundled Python file
        module_name: Name to give the loaded module

    Yields:
        The loaded module object

    Example:
        with load_bundled_module(Path("bundle.py"), "my_module") as module:
            module.some_function()
    """
    bundle_dir = str(bundle_path.parent)
    original_sys_path = sys.path.copy()

    try:
        # Add bundle directory to Python path
        if bundle_dir not in sys.path:
            sys.path.insert(0, bundle_dir)

        # Load the module dynamically
        spec = importlib.util.spec_from_file_location(module_name, bundle_path)
        if spec is None or spec.loader is None:
            raise ImportError(f"Failed to create module spec for {bundle_path}")

        module = importlib.util.module_from_spec(spec)
        sys.modules[module_name] = module
        spec.loader.exec_module(module)

        yield module

    finally:
        # Clean up sys.modules
        if module_name in sys.modules:
            del sys.modules[module_name]

        # Restore original sys.path
        sys.path[:] = original_sys_path


def get_package_requirements(package_root: Path) -> Dict[str, Set[str]]:
    """Extract requirements from a package's setup.py.

    Args:
        package_root: Root directory of the package containing setup.py

    Returns:
        Dictionary with 'install_requires' and 'extras_require' sets
    """
    setup_py = package_root / "setup.py"
    if not setup_py.exists():
        return {"install_requires": set(), "extras_require": set()}

    # Create a minimal setuptools mock to capture requirements
    requirements = {"install_requires": [], "extras_require": {}}

    def mock_setup(**kwargs):
        """Mock setup function to capture requirements."""
        if "install_requires" in kwargs:
            requirements["install_requires"] = kwargs["install_requires"]
        if "extras_require" in kwargs:
            requirements["extras_require"] = kwargs["extras_require"]

    def mock_find_packages(**kwargs):
        """Mock find_packages function."""
        return []

    # Prepare the environment
    original_sys_path = sys.path.copy()
    original_sys_argv = sys.argv.copy()
    original_modules = dict(sys.modules)

    try:
        # Change to package directory
        sys.path.insert(0, str(package_root))
        sys.argv = ["setup.py", "egg_info"]

        # Create mock setuptools module
        import types

        setuptools_mock = types.ModuleType("setuptools")
        setuptools_mock.setup = mock_setup
        setuptools_mock.find_packages = mock_find_packages
        sys.modules["setuptools"] = setuptools_mock

        # Create a namespace with our mock
        namespace = {
            "__file__": str(setup_py),
            "__name__": "__main__",
            "setup": mock_setup,
            "setuptools": setuptools_mock,
            "find_packages": mock_find_packages,
            "sys": sys,
            "os": __import__("os"),
            "open": open,
        }

        # Execute setup.py
        with open(setup_py, "r") as f:
            code = compile(f.read(), str(setup_py), "exec")
            exec(code, namespace)

        # Parse requirements to extract package names
        install_requires = set()
        for req in requirements.get("install_requires", []):
            # Extract package name (before any version specifier)
            pkg_name = req.split(">=")[0].split("==")[0].split("<")[0].split(">")[0].split("[")[0].strip()
            install_requires.add(pkg_name)

        # Collect all extras
        extras_require = set()
        for extra_reqs in requirements.get("extras_require", {}).values():
            for req in extra_reqs:
                pkg_name = req.split(">=")[0].split("==")[0].split("<")[0].split(">")[0].split("[")[0].strip()
                extras_require.add(pkg_name)

        return {"install_requires": install_requires, "extras_require": extras_require}

    except Exception as e:
        print(f"Warning: Failed to parse setup.py: {e}")
        return {"install_requires": set(), "extras_require": set()}
    finally:
        sys.path[:] = original_sys_path
        sys.argv[:] = original_sys_argv
        # Restore original modules
        if "setuptools" in original_modules:
            sys.modules["setuptools"] = original_modules["setuptools"]
        elif "setuptools" in sys.modules:
            del sys.modules["setuptools"]
