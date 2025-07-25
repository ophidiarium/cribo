"""Test cross-module attribute imports like requests.__version__"""

from my_package import utils

# This should print the version from my_package.__version__
print(f"Version from utils: {utils.get_version()}")

# Direct access to test if the issue is with function calls
print(f"Direct __version__ access: {utils.__version__}")
