"""Auth module that defines a utility function."""

from . import (
    utils,
)  # Creates circular dependency: auth -> utils -> models -> adapters -> auth


def _basic_auth_str(username, password):
    """Returns a basic auth string."""
    # Use the utils module to trigger circular import
    formatted = utils.format_credentials(username, password)
    return f"Basic {formatted}"


class HTTPBasicAuth:
    """Basic authentication class."""

    def __init__(self, username, password):
        self.username = username
        self.password = password

    def get_header(self):
        return _basic_auth_str(self.username, self.password)
