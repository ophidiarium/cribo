# Test case for forward reference issue
from helper import validate as validate_imported


def validate():
    """Local validate function that conflicts with imported one"""
    return "local validate"


def main():
    print(validate())  # Should use local validate
    print(validate_imported())  # Should use imported validate


if __name__ == "__main__":
    main()
