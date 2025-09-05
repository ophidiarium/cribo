"""Sessions module that creates more circular dependencies."""

# Import from multiple modules like requests.sessions does
from . import compat
from .compat import OrderedDict
from . import utils
from . import models


class Session:
    """Session class that depends on multiple modules."""

    def __init__(self):
        self.headers = OrderedDict() if hasattr(compat, "OrderedDict") else {}

    def request(self, data):
        """Make a request using utils."""
        return utils.process_data(data)
