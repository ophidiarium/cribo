# fmt: off
"""Entry point exercising multiline string handling across modules."""

from strings_inline import format_report
import side_effect_module
import docstring_module


def validate_docstring(obj, expected_keywords: list[str], label: str) -> None:
    """Validate that an object's docstring contains expected content.

    Args:
        obj: The object (function, class, etc.) to validate
        expected_keywords: List of strings that must appear in the docstring
        label: Human-readable label for the validation
    """
    doc = obj.__doc__
    if doc is None:
        print(f"❌ {label}: Missing docstring")
        return

    missing = [kw for kw in expected_keywords if kw not in doc]
    if missing:
        print(f"❌ {label}: Missing keywords: {missing}")
        print(f"   Docstring preview: {doc[:100]}...")
    else:
        print(f"✅ {label}: Docstring validated ({len(doc)} chars)")


def main() -> None:
    # Original multiline string tests
    data = {"name": "Cribo", "value": 42}
    print(format_report(data))
    print(side_effect_module.SUMMARY_TEXT)

    print("\n" + "=" * 60)
    print("DOCSTRING VALIDATION")
    print("=" * 60)

    # Module docstring
    validate_docstring(
        docstring_module,
        ["multiline docstrings preservation", "Indented sections", "Special characters"],
        "Module docstring"
    )

    # Function docstrings
    validate_docstring(
        docstring_module.simple_function,
        ["Single line docstring"],
        "Simple function"
    )

    validate_docstring(
        docstring_module.multiline_function,
        ["Args:", "Returns:", "Examples:"],
        "Multiline function"
    )

    validate_docstring(
        docstring_module.indented_docstring,
        ["intentional indentation", "Code example:", "def example():"],
        "Indented docstring"
    )

    # Class and method docstrings
    validate_docstring(
        docstring_module.DocumentedClass,
        ["comprehensive documentation", "Attributes:", "value:", "name:"],
        "Class docstring"
    )

    validate_docstring(
        docstring_module.DocumentedClass.__init__,
        ["Initialize", "Args:"],
        "Class __init__"
    )

    validate_docstring(
        docstring_module.DocumentedClass.get_info,
        ["formatted information", "Returns:", "A multiline string"],
        "Class method"
    )

    validate_docstring(
        docstring_module.DocumentedClass.static_method,
        ["Static method", "doesn't access instance state"],
        "Static method"
    )

    validate_docstring(
        docstring_module.DocumentedClass.class_method,
        ["Class method", "Returns:", "class name"],
        "Class method decorator"
    )

    # Special formatting docstrings
    validate_docstring(
        docstring_module.triple_quoted_variations,
        ["single triple quotes", "double quotes"],
        "Triple-quoted variations"
    )

    validate_docstring(
        docstring_module.docstring_with_formatting,
        ["╔═══════", "Unicode", "Emoji: 🎯"],
        "Formatted docstring"
    )

    # Print sample docstrings to verify content
    print("\n" + "=" * 60)
    print("SAMPLE DOCSTRING CONTENT")
    print("=" * 60)

    print("\n--- Module Docstring (first 200 chars) ---")
    print(docstring_module.__doc__[:200])

    print("\n--- Class Docstring ---")
    print(docstring_module.DocumentedClass.__doc__)

    print("\n--- Method Docstring ---")
    print(docstring_module.DocumentedClass.get_info.__doc__)

    print("\n--- Formatted Docstring Preview ---")
    print(docstring_module.docstring_with_formatting.__doc__[:300])

    # Test that methods work correctly with their docstrings
    instance = docstring_module.DocumentedClass(42, "test")
    print("\n--- Instance Method Output ---")
    print(instance.get_info())


if __name__ == "__main__":
    main()
