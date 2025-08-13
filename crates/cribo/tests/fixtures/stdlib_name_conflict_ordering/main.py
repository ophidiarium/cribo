# Test fixture that demonstrates a class dependency ordering bug
# When a package has a module named 'abc' (conflicting with stdlib),
# cribo outputs classes in wrong order causing NameError at runtime.
#
# Expected: Classes should be defined in dependency order
# Actual: Console class is output before Base class it inherits from

from complex_pkg import main

main()
