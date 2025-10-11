# fmt: off
"""Entry point exercising multiline string handling across modules."""

from strings_inline import format_report
import side_effect_module


def main() -> None:
    data = {"name": "Cribo", "value": 42}
    print(format_report(data))
    print(side_effect_module.SUMMARY_TEXT)


if __name__ == "__main__":
    main()
