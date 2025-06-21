"""Test module that tries to import pandera.polars.

This demonstrates what happens when we have a local 'pandera' package
that shadows the third-party pandera package, and we try to import
a submodule that only exists in the third-party package.
"""

print("Starting test_import.py")

try:
    # This should fail because our local pandera package doesn't have a 'polars' submodule
    import pandera.polars as pa

    print("Successfully imported pandera.polars")
    print(f"pandera.polars module: {pa.__name__}")
except ImportError as e:
    print(f"ImportError: {e}")
except AttributeError as e:
    print(f"AttributeError: {e}")

# Let's also try importing just pandera to see which one we get
try:
    import pandera

    print("\nImported pandera successfully")
    print(f"pandera.__version__: {pandera.__version__}")
    if hasattr(pandera, "local_function"):
        print(f"This is the local pandera package: {pandera.local_function()}")
    else:
        print("This is the third-party pandera package")
except ImportError as e:
    print(f"\nFailed to import pandera: {e}")
