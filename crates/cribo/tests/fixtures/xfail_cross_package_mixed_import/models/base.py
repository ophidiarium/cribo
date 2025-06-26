"""Base model module."""


class BaseModel:
    """Base model class."""

    def __init__(self, name: str):
        if not name or name.startswith("_"):
            raise ValueError(f"Invalid model name: {name}")
        self.name = name
        self.version = "1.0.0"

    def get_info(self):
        """Get model information."""
        return {
            "name": self.name,
            "type": "base",
            "model_version": self.version,
        }
