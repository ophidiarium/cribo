"""
Global Namespace Transformation Example for Cribo Bundler

This file demonstrates how Python modules with global statements
should be transformed to work correctly when wrapped in init functions.
"""

# ============================================================================
# ORIGINAL MODULE (models/base.py)
# ============================================================================

# Module-level variables
result = "base_result"
counter = 0
data = {"key": "value"}


def increment():
    """Function that modifies a global variable"""
    global counter
    counter += 1
    return counter


def initialize():
    """Function that modifies multiple globals"""
    global result, data
    result = f"initialized_{counter}"
    data["status"] = "ready"
    return result


def get_state():
    """Function that reads globals (no global declaration needed)"""
    return {"result": result, "counter": counter, "data": data}


class Processor:
    """Class with methods that use globals"""

    def process(self):
        global result
        result = f"processed_{result}"
        return result

    def reset(self):
        global counter, data
        counter = 0
        data = {"key": "value"}


# Nested scope example
def outer():
    x = "outer_local"

    def inner():
        global result  # Refers to module-level result, not outer's x
        result = "inner_modified"

    inner()
    return x  # Still "outer_local"


# ============================================================================
# TRANSFORMED MODULE (after bundling)
# ============================================================================


def __cribo_init_module():
    """Wrapper function that encapsulates the module"""

    # Step 1: Module-level variables become function-local
    result = "base_result"
    counter = 0
    data = {"key": "value"}

    # Step 2: Create module globals dictionary
    __module_globals__ = {
        "result": result,
        "counter": counter,
        "data": data,
    }

    # Step 3: Transform functions to use the globals dictionary

    def increment():
        """Transformed: global counter → dictionary access"""
        # Original: global counter
        # Original: counter += 1
        __module_globals__["counter"] += 1
        return __module_globals__["counter"]

    def initialize():
        """Transformed: multiple globals → dictionary access"""
        # Original: global result, data
        # Original: result = f"initialized_{counter}"
        __module_globals__["result"] = f"initialized_{__module_globals__['counter']}"
        # Original: data["status"] = "ready"
        __module_globals__["data"]["status"] = "ready"
        return __module_globals__["result"]

    def get_state():
        """No transformation needed - reads use dictionary"""
        return {"result": __module_globals__["result"], "counter": __module_globals__["counter"], "data": __module_globals__["data"]}

    class Processor:
        """Class methods also transformed"""

        def process(self):
            # Original: global result
            # Original: result = f"processed_{result}"
            __module_globals__["result"] = f"processed_{__module_globals__['result']}"
            return __module_globals__["result"]

        def reset(self):
            # Original: global counter, data
            __module_globals__["counter"] = 0
            __module_globals__["data"] = {"key": "value"}

    def outer():
        x = "outer_local"

        def inner():
            # Original: global result
            __module_globals__["result"] = "inner_modified"

        inner()
        return x

    # Step 4: Create module object and expose values
    import types

    module = types.ModuleType("__cribo_wrapped_module")

    # Expose functions and classes
    module.increment = increment
    module.initialize = initialize
    module.get_state = get_state
    module.Processor = Processor
    module.outer = outer

    # Expose module-level variables (with current values)
    module.result = __module_globals__["result"]
    module.counter = __module_globals__["counter"]
    module.data = __module_globals__["data"]

    # Optional: Expose globals dict for debugging
    module.__module_globals__ = __module_globals__

    return module


# ============================================================================
# ALTERNATIVE: GLOBALS LIFTING APPROACH
# ============================================================================

# Instead of dictionary, lift globals to true module level with unique names

# Generated at true module level
__cribo_base_result = "base_result"
__cribo_base_counter = 0
__cribo_base_data = {"key": "value"}


def __cribo_init_module_v2():
    """Alternative approach using lifted globals"""

    def increment():
        global __cribo_base_counter
        __cribo_base_counter += 1
        return __cribo_base_counter

    def initialize():
        global __cribo_base_result, __cribo_base_data
        __cribo_base_result = f"initialized_{__cribo_base_counter}"
        __cribo_base_data["status"] = "ready"
        return __cribo_base_result

    # ... rest of implementation


# ============================================================================
# EDGE CASES TO HANDLE
# ============================================================================


def edge_cases():
    """Examples of tricky global usage patterns"""

    # 1. Global declaration without assignment
    global future_var
    # Should track but not add to initial globals dict

    # 2. Conditional global usage
    if some_condition:
        global conditional_var
        conditional_var = "value"

    # 3. Global with del statement
    global deletable
    del deletable  # Must handle KeyError

    # 4. Global with augmented assignment
    global counter
    counter += 1  # Transform to: __module_globals__['counter'] += 1

    # 5. Global in comprehension
    [x for x in range(10) if (global_var := x) > 5]  # Python 3.8+ walrus

    # 6. Global in exception handler
    try:
        risky_operation()
    except Exception as e:
        global error_count
        error_count += 1


# ============================================================================
# CROSS-MODULE GLOBAL ACCESS
# ============================================================================


def cross_module_example():
    """
    When module A accesses globals from module B after bundling
    """
    # Original:
    # from other_module import shared_global
    # shared_global = "modified"

    # Transformed:
    # other_module = sys.modules['__cribo_other_module']
    # other_module.__module_globals__['shared_global'] = "modified"
    pass


# ============================================================================
# TESTING THE TRANSFORMATION
# ============================================================================


def test_transformed_module():
    """Verify the transformation preserves semantics"""

    # Initialize module
    module = __cribo_init_module()

    # Test 1: Initial state
    assert module.get_state() == {"result": "base_result", "counter": 0, "data": {"key": "value"}}

    # Test 2: Increment modifies global
    assert module.increment() == 1
    assert module.increment() == 2

    # Test 3: Initialize uses current counter
    assert module.initialize() == "initialized_2"
    assert module.get_state()["data"]["status"] == "ready"

    # Test 4: Class method modifies global
    processor = module.Processor()
    assert processor.process() == "processed_initialized_2"

    # Test 5: Nested scope modifies correct global
    assert module.outer() == "outer_local"
    assert module.__module_globals__["result"] == "inner_modified"

    print("All tests passed!")


if __name__ == "__main__":
    test_transformed_module()
