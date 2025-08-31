"""Models module that creates part of the circular chain."""

# Import adapters to create circular dependency
# models -> adapters -> auth -> utils -> models
from . import adapters as _adapters_module


class BaseModel:
    """Base model class."""

    def __init__(self):
        self.model_type = "base"

    def get_adapter_class(self):
        """Reference to adapter class (creates circular dependency)."""
        return (
            _adapters_module.Adapter if hasattr(_adapters_module, "Adapter") else None
        )
