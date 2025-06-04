"""
Config module with global variables.
These will conflict with constants.py when bundled.
"""

# API configuration (conflicts with constants.py)
API_URL = "https://api.config.com"
VERSION = "2.1.0"

# Additional config
MAX_RETRIES = 3
CACHE_SIZE = 1000
