"""Entry module with a child that imports back from it (circular dependency)."""

# Import child module that will import back from us
import child_with_init


def get_console():
    """Function that should be accessible from child module."""
    return "console object"


def main():
    """Main function."""
    obj = child_with_init.ConsoleUser()
    return obj.use_console()


if __name__ == "__main__":
    print(f"Result: {main()}")
