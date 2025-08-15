# Import with alias
from .base import YAMLObject as YO
from .meta import YAMLObjectMetaclass

# Re-export with alias
__all__ = ["YO", "YAMLObjectMetaclass"]
