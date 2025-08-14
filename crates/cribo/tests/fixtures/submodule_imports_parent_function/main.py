"""Test where submodule imports function from parent package.

This test works in Python but fails when bundled because:
1. pkg.submodule imports get_base from parent at module level
2. pkg.submodule uses the imported function at module level
3. The bundler fails to properly initialize pkg.submodule
4. When get_result() tries to access pkg.submodule.process(), it fails
"""

import pkg

# This should work in Python but fail when bundled
print(pkg.get_result())
