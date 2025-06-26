"""Module demonstrating globals() dictionary access."""

# Module-level global variable
bar = "module2_bar"
baz = 42


def get_bar():
    """Get bar using globals() dict."""
    return globals()["bar"]


def modify_bar():
    """Modify bar using globals() dict."""
    globals()["bar"] = "module2_bar_modified"


def set_dynamic_global(name, value):
    """Set a global variable dynamically using globals()."""
    globals()[name] = value


def get_dynamic_global(name, default=None):
    """Get a global variable dynamically."""
    return globals().get(name, default)


def list_module_globals():
    """List all non-built-in globals in this module."""
    return {
        k: v
        for k, v in globals().items()
        if not k.startswith("__")
        and k
        not in [
            "get_bar",
            "modify_bar",
            "set_dynamic_global",
            "get_dynamic_global",
            "list_module_globals",
        ]
    }


# Create some dynamic globals
set_dynamic_global("dynamic1", "created_via_globals")
set_dynamic_global("dynamic2", [1, 2, 3])
