"""Config module with side effects (triggers wrapper) and conditional wildcard import."""

# Side effect: module-level function call forces this into a wrapper module
print("config module loaded")

try:
    from defaults import *
except ImportError:
    MAX_RETRIES = 1
    DEFAULT_TIMEOUT = 60
    API_VERSION = "v1"


def get_config():
    return {
        "max_retries": MAX_RETRIES,
        "timeout": DEFAULT_TIMEOUT,
        "api_version": API_VERSION,
    }
