"""External plugin that registers itself when imported."""

from state import add_plugin


def register_plugin():
    """Register this plugin."""
    return add_plugin("external_plugin")


# Side effect: automatically register when imported
add_plugin("external_plugin_loaded")
