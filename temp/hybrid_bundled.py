import sys, types


def __cribo_init___cribo_341a42_main():
    if "__cribo_341a42_main" in sys.modules:
        return sys.modules["__cribo_341a42_main"]
    module = types.ModuleType("__cribo_341a42_main")
    module.__file__ = "crates/cribo/tests/fixtures/bundling/all_variable_handling/main.py"
    sys.modules["__cribo_341a42_main"] = module
    sys.modules["main"] = module
    print("Testing simple module exports:")
    print(f"public_func() = {public_func()}")
    print(f"CONSTANT = {CONSTANT}")
    print("\nTesting nested package exports:")
    print(f"exported_from_init() = {exported_from_init()}")
    print(f"sub_function() = {sub_function()}")
    print("\nTesting conflict resolution:")
    print(f"message = {message}")
    print(f"\nsimple_module.__all__ = {simple_module.__all__}")
    print(f"submodule.__all__ = {sub.__all__}")
    return module


def __cribo_init___cribo_ee784c_simple_module():
    if "__cribo_ee784c_simple_module" in sys.modules:
        return sys.modules["__cribo_ee784c_simple_module"]
    module = types.ModuleType("__cribo_ee784c_simple_module")
    module.__file__ = "/Volumes/workplace/GitHub/ophidiarium/cribo/crates/cribo/tests/fixtures/bundling/all_variable_handling/simple_module.py"
    sys.modules["__cribo_ee784c_simple_module"] = module
    sys.modules["simple_module"] = module
    __all__ = ["public_func", "CONSTANT"]
    module.__all__ = __all__

    def public_func():
        """A public function that should be exported."""
        return "Hello from public_func"

    module.public_func = public_func

    def _private_func():
        """A private function that should not be exported."""
        return "This is private"

    module._private_func = _private_func
    CONSTANT = 42
    module.CONSTANT = CONSTANT
    _PRIVATE_CONSTANT = "secret"
    module._PRIVATE_CONSTANT = _PRIVATE_CONSTANT

    class InternalClass:
        pass

    module.InternalClass = InternalClass
    return module


def __cribo_init___cribo_2657f2_nested_package():
    if "__cribo_2657f2_nested_package" in sys.modules:
        return sys.modules["__cribo_2657f2_nested_package"]
    module = types.ModuleType("__cribo_2657f2_nested_package")
    module.__file__ = "/Volumes/workplace/GitHub/ophidiarium/cribo/crates/cribo/tests/fixtures/bundling/all_variable_handling/nested_package/__init__.py"
    sys.modules["__cribo_2657f2_nested_package"] = module
    sys.modules["nested_package"] = module
    from .submodule import sub_function
    from .utils import helper_func

    __all__ = ["exported_from_init", "sub_function"]
    module.__all__ = __all__

    def exported_from_init():
        """Function exported from package __init__.py"""
        return f"From init, using helper: {helper_func()}"

    module.exported_from_init = exported_from_init

    def _internal_init_func():
        """Internal function not exported"""
        return "internal"

    module._internal_init_func = _internal_init_func
    PACKAGE_CONSTANT = "from_package"
    module.PACKAGE_CONSTANT = PACKAGE_CONSTANT
    return module


def __cribo_init___cribo_e9952d_nested_package_submodule():
    if "__cribo_e9952d_nested_package_submodule" in sys.modules:
        return sys.modules["__cribo_e9952d_nested_package_submodule"]
    module = types.ModuleType("__cribo_e9952d_nested_package_submodule")
    module.__file__ = "/Volumes/workplace/GitHub/ophidiarium/cribo/crates/cribo/tests/fixtures/bundling/all_variable_handling/nested_package/submodule.py"
    sys.modules["__cribo_e9952d_nested_package_submodule"] = module
    sys.modules["nested_package.submodule"] = module
    __all__ = ["sub_function", "SUB_CONSTANT"]
    module.__all__ = __all__

    def sub_function():
        """Function from submodule"""
        return "Hello from submodule"

    module.sub_function = sub_function

    def _private_sub_func():
        """Private function in submodule"""
        return "private submodule function"

    module._private_sub_func = _private_sub_func
    SUB_CONSTANT = "submodule_value"
    module.SUB_CONSTANT = SUB_CONSTANT
    message = "from submodule"
    module.message = message
    return module


def __cribo_init___cribo_62e3ba_nested_package_utils():
    if "__cribo_62e3ba_nested_package_utils" in sys.modules:
        return sys.modules["__cribo_62e3ba_nested_package_utils"]
    module = types.ModuleType("__cribo_62e3ba_nested_package_utils")
    module.__file__ = "/Volumes/workplace/GitHub/ophidiarium/cribo/crates/cribo/tests/fixtures/bundling/all_variable_handling/nested_package/utils.py"
    sys.modules["__cribo_62e3ba_nested_package_utils"] = module
    sys.modules["nested_package.utils"] = module

    def helper_func():
        """Helper function used by other modules"""
        return "helper result"

    module.helper_func = helper_func

    def another_helper():
        """Another helper function"""
        return "another helper"

    module.another_helper = another_helper
    UTILS_CONSTANT = "utils value"
    module.UTILS_CONSTANT = UTILS_CONSTANT
    return module


__cribo_modules = {"main": "__cribo_341a42_main", "simple_module": "__cribo_ee784c_simple_module", "nested_package": "__cribo_2657f2_nested_package", "nested_package.submodule": "__cribo_e9952d_nested_package_submodule", "nested_package.utils": "__cribo_62e3ba_nested_package_utils"}
__cribo_init_functions = {"__cribo_341a42_main": __cribo_init___cribo_341a42_main, "__cribo_ee784c_simple_module": __cribo_init___cribo_ee784c_simple_module, "__cribo_2657f2_nested_package": __cribo_init___cribo_2657f2_nested_package, "__cribo_e9952d_nested_package_submodule": __cribo_init___cribo_e9952d_nested_package_submodule, "__cribo_62e3ba_nested_package_utils": __cribo_init___cribo_62e3ba_nested_package_utils}


class CriboBundledFinder:
    def __init__(self, module_registry, init_functions):
        self.module_registry = module_registry
        self.init_functions = init_functions

    def find_spec(self, fullname, path, *, target=None):
        if fullname in self.module_registry:
            synthetic_name = self.module_registry[fullname]
            if synthetic_name not in sys.modules:
                init_func = self.init_functions.get(synthetic_name)
                if init_func:
                    init_func()
            import importlib.util

            return importlib.util.find_spec(synthetic_name)
        return None


sys.meta_path.insert(0, CriboBundledFinder(__cribo_modules, __cribo_init_functions))
__cribo_init___cribo_341a42_main()
__cribo_init___cribo_ee784c_simple_module()
__cribo_init___cribo_2657f2_nested_package()
__cribo_init___cribo_e9952d_nested_package_submodule()
__cribo_init___cribo_62e3ba_nested_package_utils()
__all__ = ["message", "SHARED_NAME"]
message = "from conflict_module"
SHARED_NAME = "conflict_module_version"


def internal_func():
    """Internal function not in __all__"""
    return "internal"


__all__backup = "this is not the real __all__"
