"""Test fixture for parameter names shadowing global/class names.

This fixture demonstrates the pattern where function parameters shadow
important module-level names (classes, functions, constants).
"""

from models import User, Connection


# Global variable that will be shadowed
result = "global_result"


def process_user(User: "User") -> str:
    """Function with parameter shadowing the imported User class."""
    # Here 'User' refers to the parameter, not the imported class
    return f"Processing: {User.name}"


def create_connection(Connection: str) -> str:
    """Function with parameter shadowing the imported Connection class."""
    # Here 'Connection' refers to the string parameter
    return f"Connection string: {Connection}"


def manipulate_result(result: int) -> int:
    """Function with parameter shadowing the global variable."""
    # Here 'result' refers to the parameter, not the global
    return result * 2


class Manager:
    """Class with methods that have shadowing parameters."""

    def __init__(self):
        self.users = []
        self.connections = []

    def add_user(self, User: "User") -> None:
        """Method with parameter shadowing the User class."""
        # 'User' is the parameter, not the class
        self.users.append(User)
        print(f"Added user: {User.name}")

    def add_connection(self, Connection: "Connection") -> None:
        """Method with parameter shadowing the Connection class."""
        # 'Connection' is the parameter, not the class
        self.connections.append(Connection)
        print(f"Added connection: {Connection.host}")

    def process_with_shadow(self, result: str) -> str:
        """Method with parameter shadowing the global variable."""
        # 'result' is the parameter, not the global
        global_val = globals()["result"]  # Access the actual global
        return f"Param: {result}, Global: {global_val}"


def complex_shadowing(User: type, Connection: callable, result: list) -> dict:
    """Function with multiple shadowing parameters of different types."""
    # All parameters shadow their module-level counterparts
    return {
        "user_type": User.__name__ if hasattr(User, "__name__") else str(User),
        "connection_callable": Connection() if callable(Connection) else None,
        "result_list": len(result) if isinstance(result, list) else 0,
    }


def main():
    """Demonstrate the shadowing patterns."""
    # Create real instances using the imported classes
    user = User("Alice")
    conn = Connection("localhost", 5432)

    # Call functions with shadowing parameters
    print(process_user(user))  # Pass User instance to User parameter
    print(
        create_connection("postgres://localhost")
    )  # Pass string to Connection parameter
    print(manipulate_result(10))  # Pass int to result parameter

    # Use the Manager class
    manager = Manager()
    manager.add_user(user)
    manager.add_connection(conn)
    print(manager.process_with_shadow("parameter_value"))

    # Complex shadowing with different types
    shadow_result = complex_shadowing(
        User=type("DynamicUser", (), {}),
        Connection=lambda: "dynamic_connection",
        result=[1, 2, 3],
    )
    print(f"Complex shadowing result: {shadow_result}")

    # Verify globals are still accessible
    print(f"Global result value: {result}")
    print(f"User class accessible: {User}")
    print(f"Connection class accessible: {Connection}")


if __name__ == "__main__":
    main()
