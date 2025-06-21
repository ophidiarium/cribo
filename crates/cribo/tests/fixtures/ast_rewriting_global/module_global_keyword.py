"""Module demonstrating global keyword usage."""

# Module-level global variable
foo = "module1_foo"


def get_foo():
    """Get the module's global foo value."""
    return foo


def modify_foo():
    """Modify the module's global foo using global keyword."""
    global foo
    foo = "module1_foo_modified"


def set_foo_with_global(value):
    """Set foo using global keyword."""
    global foo
    foo = value


# Function that creates a local foo then accesses global
def local_vs_global():
    """Demonstrate local vs global scope."""
    foo = "local_foo"  # Local variable

    def inner():
        global foo
        return foo  # Returns the module's global foo

    return (foo, inner())  # Returns (local_foo, module's global foo)
