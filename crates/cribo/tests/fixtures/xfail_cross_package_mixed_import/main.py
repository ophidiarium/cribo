"""Test fixture demonstrating cross-package mixed import patterns.

This tests the specific pattern where a module uses both:
1. Deep absolute imports crossing package boundaries (e.g., from models.user)
2. Relative imports within its package (e.g., from ..utils.helpers)
3. Import order dependencies with __init__.py files
"""

# First, import from core which triggers cross-package imports
from core import initialize_core
from core.database import connect as db_connect

# Import specific submodule functionality
from core.database.connection import connect, get_connection_info, CONNECTION_METADATA

# Import from models to show circular dependency handling
from models import get_model_version, DEFAULT_MODEL_CONFIG, HAS_ADVANCED

# Import package-level re-exports
from core import validate, get_config


def demonstrate_import_patterns():
    """Demonstrate various import pattern behaviors."""
    print("=== Import Pattern Demonstration ===")

    # Show import-time computations
    print(f"\n1. Import-time values:")
    print(f"   - Model version from models package: {get_model_version()}")
    print(f"   - Core model version: {CONNECTION_METADATA['core_version']}")
    print(f"   - Model config features: {DEFAULT_MODEL_CONFIG['features']}")
    print(f"   - Has advanced model: {HAS_ADVANCED}")

    # Show that imports work before initialization
    print(f"\n2. Pre-initialization state:")
    print(f"   - Config before init: {get_config()}")

    # Initialize and show state change
    initialize_core(debug=True)
    print(f"\n3. Post-initialization state:")
    print(f"   - Config after init: {get_config()}")

    # Test connections using different import paths
    print(f"\n4. Testing connections:")

    # Direct import
    conn1 = connect("test_db")
    print(f"   - Direct import: {conn1}")

    # Package-level import
    conn2 = db_connect("prod_db")
    print(f"   - Package-level import: {conn2}")

    # Model-prefixed to trigger lazy import
    conn3 = connect("model_user_db")
    print(f"   - With lazy import: {conn3}")

    # Get comprehensive info
    info = get_connection_info()
    print(f"\n5. Connection info shows all imports:")
    print(f"   - Debug mode: {info['debug_mode']}")
    print(f"   - Available validators: {info['available_validators']}")
    print(f"   - Metadata processor: {info['metadata']['processor']}")
    print(f"   - Config included: {'config' in info}")

    return info


def main():
    """Main entry point."""
    results = demonstrate_import_patterns()

    # Show that package-level validator works
    print(f"\n6. Package-level re-exports:")
    print(f"   - validate('test'): {validate('test')}")
    print(f"   - validate('_hidden'): {validate('_hidden')}")

    return results


if __name__ == "__main__":
    result = main()
    print(f"\n=== Final Result ===")
    print(f"Successfully demonstrated cross-package mixed import patterns")
