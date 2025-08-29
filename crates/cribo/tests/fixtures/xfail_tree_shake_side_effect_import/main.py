# Entry point that triggers the tree-shaking bug
# When a package with side effects imports and re-exports symbols,
# those symbols get incorrectly tree-shaken away

import mypackage

# This should trigger module initialization
print("Test completed")
