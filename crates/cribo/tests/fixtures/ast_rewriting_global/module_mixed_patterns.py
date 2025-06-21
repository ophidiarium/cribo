"""Module demonstrating mixed global access patterns."""

# Module globals
foo = "module3_foo"
bar = "module3_bar"
counter = 0


def get_values():
    """Get module globals using different methods."""
    # Direct access
    direct_foo = foo

    # Via globals() dict
    dict_bar = globals()["bar"]

    # Via globals().get() with default
    counter_val = globals().get("counter", -1)

    return {"foo": direct_foo, "bar": dict_bar, "counter": counter_val}


def modify_all():
    """Modify globals using different patterns."""
    global foo, counter

    # Modify using global keyword
    foo = "module3_foo_modified"
    counter += 1

    # Modify using globals() dict
    globals()["bar"] = "module3_bar_modified"

    # Add new global dynamically
    globals()["new_var"] = "dynamically_added"


def complex_global_usage():
    """Demonstrate complex global usage patterns."""
    global counter

    # Read global
    original = counter

    # Modify in loop
    for i in range(3):
        counter += 1

    # Create closure that captures global
    def increment():
        global counter
        counter += 1
        return counter

    # Use globals() in comprehension
    global_keys = [k for k in globals() if not k.startswith("_")]

    return {
        "original": original,
        "after_loop": counter,
        "increment_func": increment,
        "global_count": len(global_keys),
    }


# Initialize some module state
globals()["initialized_via_globals"] = True
exec("exec_created_var = 'created_via_exec'", globals())
