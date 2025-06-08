#!/usr/bin/env python3
"""
Variable name collision test.
Both modules define variables with the same names.
"""

from constants import API_URL, VERSION, DEFAULT_TIMEOUT
from config import API_URL as CONFIG_API_URL, VERSION as CONFIG_VERSION, MAX_RETRIES


def main():
    # Use variables from both modules
    print(f"Constants API URL: {API_URL}")
    print(f"Config API URL: {CONFIG_API_URL}")
    print(f"Constants Version: {VERSION}")
    print(f"Config Version: {CONFIG_VERSION}")
    print(f"Default Timeout: {DEFAULT_TIMEOUT}")
    print(f"Max Retries: {MAX_RETRIES}")

    return {"constants_api_url": API_URL, "config_api_url": CONFIG_API_URL, "constants_version": VERSION, "config_version": CONFIG_VERSION, "default_timeout": DEFAULT_TIMEOUT, "max_retries": MAX_RETRIES}


if __name__ == "__main__":
    result = main()
    print("Result:", result)
