# This mimics PyYAML's __init__.py pattern

# Import classes in specific order
from .base import YAMLObject
from .meta import YAMLObjectMetaclass

# Re-export for public API
__all__ = ["YAMLObject", "YAMLObjectMetaclass"]
