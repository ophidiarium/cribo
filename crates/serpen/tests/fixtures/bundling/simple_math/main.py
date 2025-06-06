from calculator import add, multiply
from utils import format_result


def main():
    # Simple math operations
    result1 = add(5, 3)
    result2 = multiply(4, 7)

    print(format_result("Addition", result1))
    print(format_result("Multiplication", result2))


if __name__ == "__main__":
    main()
