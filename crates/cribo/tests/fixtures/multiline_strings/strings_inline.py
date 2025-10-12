# fmt: off
"""Inline-only module mixing multiline literal patterns."""

HEADER = '''Inline Section
- maintains indentation
- uses single quotes
'''

DOUBLE_QUOTED = """Double-quoted block demonstrating nested 'single quotes'
and maintaining the original quoting style.
"""

TAIL = (
    """Trailing piece
    spans multiple
    lines
    """
    "with extra info\n"
)


def format_report(data: dict[str, object]) -> str:
    """Combine multiline pieces into final string."""
    body = f"""Report:
    name={data['name']}
    value={data['value']}
    """
    extra = f'''Summary:
    keys={", ".join(data.keys())}
    '''
    return HEADER + DOUBLE_QUOTED + body + TAIL + extra
