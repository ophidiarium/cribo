# Package __init__.py that imports utils module
# The bug occurs because utils has side effects and imports CustomError,
# but CustomError gets incorrectly tree-shaken away

from . import utils

# Package initialization
print("mypackage initialized")
