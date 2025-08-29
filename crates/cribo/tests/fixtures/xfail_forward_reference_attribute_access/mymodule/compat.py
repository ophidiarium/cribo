# Compat module that provides JSONDecodeError
import json

# Export the JSON decoder error for compatibility
JSONDecodeError = json.JSONDecodeError

__all__ = ["JSONDecodeError"]
