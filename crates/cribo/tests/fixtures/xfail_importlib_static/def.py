"""Module named 'def' - another Python keyword."""


class FunctionDefinition:
    def __init__(self, func_name, params=None):
        self.name = func_name
        self.params = params or []

    def __str__(self):
        params_str = ", ".join(self.params)
        return f"def {self.name}({params_str})"


def define_function(name, *args):
    return FunctionDefinition(name, list(args))


BUILTIN_FUNCTIONS = ["print", "len", "range", "sorted"]
DEFINITION_TEMPLATE = "def {}(): pass"
