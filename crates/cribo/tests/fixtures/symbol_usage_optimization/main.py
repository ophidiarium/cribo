"""Test that unused imports in function bodies are not initialized."""

from submodule import process_data


def main():
    # process_data imports helpers but only uses one symbol
    result = process_data("test")
    print(result)


if __name__ == "__main__":
    main()
