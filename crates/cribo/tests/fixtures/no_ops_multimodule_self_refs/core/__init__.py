"""Core package with self-references."""

from .processor import DataProcessor, transform
from .config import CONFIG, Settings

# Package-level self-references (should be removed)
DataProcessor = DataProcessor  # Should be removed
transform = transform  # Should be removed
CONFIG = CONFIG  # Should be removed
Settings = Settings  # Should be removed

# Re-export with self-reference
__all__ = ["DataProcessor", "transform", "CONFIG", "Settings"]
__all__ = __all__  # Should be removed
