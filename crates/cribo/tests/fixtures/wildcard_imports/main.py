"""Main module demonstrating various wildcard import patterns."""

# Pattern 1: Wildcard import from module with explicit __all__ and no side-effects
from explicit_all import *

# Pattern 2: Wildcard import from module with side-effects and no __all__
from with_side_effects import *

# Pattern 3: Wildcard imports with name clashes
# The second import will override symbols from the first
from clash_module_a import *
from clash_module_b import *  # This will override shared_function, SharedClass, SHARED_CONSTANT


def main():
    """Test all imported symbols."""
    results = []

    # From explicit_all module (respects __all__)
    results.append(f"safe_function: {safe_function()}")
    results.append(f"SafeClass: {SafeClass().method()}")
    results.append(f"SAFE_CONSTANT: {SAFE_CONSTANT}")

    # These should NOT be available (not in __all__)
    try:
        results.append(f"_private_function: {_private_function()}")
    except NameError:
        results.append("_private_function: correctly not imported")

    try:
        results.append(f"PRIVATE_CONSTANT: {PRIVATE_CONSTANT}")
    except NameError:
        results.append("PRIVATE_CONSTANT: correctly not imported")

    # From with_side_effects module (no __all__, imports everything public)
    results.append(f"effect_function: {effect_function()}")
    results.append(f"EffectClass: {EffectClass().method()}")
    results.append(f"EFFECT_CONSTANT: {EFFECT_CONSTANT}")
    results.append(f"registered_function: {registered_function()}")

    # From clash modules - should have module_b's versions (imported last)
    results.append(f"shared_function: {shared_function()}")  # Should be from module_b
    results.append(f"SharedClass: {SharedClass().method()}")  # Should be from module_b
    results.append(f"SHARED_CONSTANT: {SHARED_CONSTANT}")  # Should be from module_b

    # Unique functions from both modules should be available
    results.append(f"unique_a_function: {unique_a_function()}")
    results.append(f"unique_b_function: {unique_b_function()}")

    return results


if __name__ == "__main__":
    print("Testing wildcard imports:")
    print("-" * 40)
    for result in main():
        print(result)
    print("-" * 40)
    print("All tests completed!")
