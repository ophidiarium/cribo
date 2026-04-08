#!/usr/bin/env python3
"""
Test that conditional wildcard imports (from x import *) inside try
blocks in wrapper modules do not produce invalid code like `module.* = *`.
"""

from config import get_config

result = get_config()
print(f"max_retries={result['max_retries']}")
print(f"timeout={result['timeout']}")
print(f"api_version={result['api_version']}")
