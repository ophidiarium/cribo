"""Models module to add more complexity."""

from . import utils
from .compat import basestring


class Request:
    """Request model."""

    def __init__(self, data):
        self.data = data
