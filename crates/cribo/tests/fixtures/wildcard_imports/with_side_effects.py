"""Module with side-effects and no explicit __all__."""

# Side-effect: print statement at module level
print("Loading with_side_effects module")

# Side-effect: global state modification
_module_state = []
_module_state.append("initialized")


def effect_function():
    """Function from module with side-effects."""
    return "effect_function_result"


class EffectClass:
    """Class from module with side-effects."""

    instances = []  # Class-level state

    def __init__(self):
        self.instances.append(self)

    def method(self):
        return "EffectClass.method_result"


EFFECT_CONSTANT = "EFFECT_VALUE"

# Side-effect: registering something globally
_registry = {}


def register(name):
    """Decorator with side-effects."""

    def decorator(func):
        _registry[name] = func
        return func

    return decorator


@register("sample")
def registered_function():
    """Function registered as a side-effect."""
    return "registered_result"


# Another side-effect at module level
if len(_module_state) > 0:
    print(f"Module state initialized with {len(_module_state)} items")
