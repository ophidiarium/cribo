# Simple module with __all__ defining exports
__all__ = ["public_func", "CONSTANT"]


def public_func():
    """A public function that should be exported."""
    return "Hello from public_func"


def _private_func():
    """A private function that should not be exported."""
    return "This is private"


CONSTANT = 42

_PRIVATE_CONSTANT = "secret"


# This should be accessible when module is imported directly
class InternalClass:
    pass
