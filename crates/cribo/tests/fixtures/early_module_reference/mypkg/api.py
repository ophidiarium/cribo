"""API module that references sessions module early.

This reproduces the issue where a module-level variable references
another module before it's fully initialized.
"""

from . import sessions

# This line causes the issue - it tries to access sessions module
# at module level before sessions has been fully initialized
session_ref = sessions  # Save reference for later use


def request(method, url, **kwargs):
    """Make a request using the session."""
    # This would fail if sessions is not properly initialized
    with session_ref.Session() as session:
        return session.request(method=method, url=url, **kwargs)
