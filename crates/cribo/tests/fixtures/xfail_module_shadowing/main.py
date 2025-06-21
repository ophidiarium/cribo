"""Main entry point that imports test_import to demonstrate module shadowing behavior."""

print("=== Module Shadowing Test ===")
print("This test demonstrates Python's behavior when a first-party module")
print("shadows a third-party package name.\n")

# Import the test module which will try to import pandera.polars
import test_import

print("\n=== Test Complete ===")
