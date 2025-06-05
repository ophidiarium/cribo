"""Test case with class-level circular dependencies."""

from user_class import User
from admin_class import Admin


def main():
    user = User("Alice")
    admin = Admin("Bob")
    print(f"User: {user.name}")
    print(f"Admin: {admin.name}")


if __name__ == "__main__":
    main()
