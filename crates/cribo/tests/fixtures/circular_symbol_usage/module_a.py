"""Module A with circular dependency on B."""

from typing import Optional
from module_b import HelperB, UnusedFromB


class FunctionA:
    def compute(self) -> str:
        # Only HelperB is used, UnusedFromB should not be initialized
        helper = HelperB()
        return f"A: {helper.get_value()}"

    def unused_method(self, param: Optional["UnusedFromB"]) -> None:
        # UnusedFromB is only in type annotation, not runtime
        pass
