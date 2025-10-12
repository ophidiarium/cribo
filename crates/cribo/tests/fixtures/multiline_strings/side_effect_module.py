# fmt: off
"""Module with side effects and diverse multiline literals."""

START_BANNER = """Loading side effect module:
    - timestamps
    - values
"""

DETAILS = (
    '''        Captured context:
        module={__name__}
    '''
    "        status=ready"
)


def _render_start() -> str:
    """Produce a multiline message with runtime data."""
    message = f"""Start Time:
    2024-01-02 03:04:05
    Module: multiline_strings.side_effect_module
    Status: {"ready"}
    """
    more = """Computed Values:
        length=62
    """
    note = (
        """Generated output
        includes a blank line
        """
        "appended"
    )
    return message + more + note


SUMMARY_TEXT = START_BANNER + DETAILS

print(_render_start())
