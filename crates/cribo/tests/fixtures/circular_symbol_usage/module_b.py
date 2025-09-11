"""Module B with circular dependency on A."""

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from module_a import FunctionA


class HelperB:
    def get_value(self) -> str:
        return "Value from B"


class UnusedFromB:
    def __init__(self):
        print("UnusedFromB initialized - this should be dead code!")

    def process(self, func_a: "FunctionA") -> str:
        # Creates circular dependency
        return func_a.compute()
