"""Pytest configuration for ecosystem tests.

This file configures pytest settings for all ecosystem tests, including:
- Network request timeouts
- Test execution timeouts
"""

import os
import pytest


def pytest_configure(config):
    """Configure pytest with custom settings for ecosystem tests."""
    # Add custom markers
    config.addinivalue_line("markers", "network: mark test as requiring network access")
    config.addinivalue_line("markers", "slow: mark test as slow running")


def pytest_collection_modifyitems(config, items):
    """Add markers to tests based on their characteristics."""
    for item in items:
        # Mark all httpx and requests tests as network tests
        if "httpx" in item.nodeid or "requests" in item.nodeid:
            item.add_marker(pytest.mark.network)

        # Mark specific slow tests
        if "test_bundled_timeout" in item.nodeid or "test_bundled_async" in item.nodeid:
            item.add_marker(pytest.mark.slow)
