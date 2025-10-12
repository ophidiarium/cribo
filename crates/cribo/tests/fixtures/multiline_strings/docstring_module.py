# fmt: off
"""Module demonstrating multiline docstrings preservation.

This docstring spans multiple lines and contains:
    - Indented sections
    - Special characters: "quotes", 'apostrophes', and symbols !@#$%
    - Empty lines above
"""


def simple_function():
    """Single line docstring for simple function."""
    return "simple"


def multiline_function(x: int, y: int) -> int:
    """Calculate the sum of two numbers.

    Args:
        x: First number
        y: Second number

    Returns:
        The sum of x and y

    Examples:
        >>> multiline_function(1, 2)
        3
    """
    return x + y


def indented_docstring():
    """
    This docstring has intentional indentation patterns:
        - Indented list item 1
        - Indented list item 2
            - Nested item

    Code example:
        def example():
            return True

    More text after code block.
    """
    return "indented"


class DocumentedClass:
    """A class with comprehensive documentation.

    This class demonstrates various docstring patterns:
        - Class-level documentation
        - Method documentation
        - Special method documentation

    Attributes:
        value: An integer value
        name: A string identifier
    """

    def __init__(self, value: int, name: str):
        """Initialize the DocumentedClass.

        Args:
            value: The initial value
            name: The name identifier
        """
        self.value = value
        self.name = name

    def get_info(self) -> str:
        """Return formatted information about this instance.

        Returns:
            A multiline string containing:
                - The instance name
                - The instance value
                - Additional metadata
        """
        return f"""Instance Info:
    name={self.name}
    value={self.value}
    type={type(self).__name__}
"""

    @staticmethod
    def static_method():
        """Static method with docstring.

        This method doesn't access instance state.
        It exists to demonstrate static method docstrings.
        """
        return "static"

    @classmethod
    def class_method(cls):
        """Class method with docstring.

        Returns:
            The class name as a string.
        """
        return cls.__name__


def triple_quoted_variations():
    '''This docstring uses single triple quotes.

    It should be preserved identically to double-quoted docstrings.
    Special characters: "double quotes" work fine here.
    '''
    return "single-triple-quoted"


def docstring_with_formatting():
    """
    ╔═══════════════════════════════════╗
    ║  Fancy Box-Drawing Characters     ║
    ╠═══════════════════════════════════╣
    ║  - Unicode support                ║
    ║  - Special formatting             ║
    ║  - Emoji: 🎯 ✅ 🚀                ║
    ╚═══════════════════════════════════╝

    This tests preservation of:
        • Unicode box-drawing characters
        • Emoji and special symbols
        • Mixed indentation levels
    """
    return "formatted"


CONSTANT_WITH_DOCSTRING = 42
"""This is a module-level constant docstring.

Constants can have docstrings too, though they're less common.
This tests that such docstrings are preserved.
"""
