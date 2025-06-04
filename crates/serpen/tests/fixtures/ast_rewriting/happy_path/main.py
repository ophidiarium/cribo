#!/usr/bin/env python3
"""
Happy path main module that uses multiple nested modules without name collisions.
This should bundle cleanly without any AST rewriting for conflicts.
"""

from utils.helpers import format_message, calculate_total
from models.user import User, UserRole
from services.database import DatabaseService


def main():
    # Create a user
    user = User("Alice", "alice@example.com", UserRole.ADMIN)

    # Create database service
    db = DatabaseService("test.db")

    # Use utility functions
    message = format_message("Welcome", user.name)
    total = calculate_total([10, 20, 30])

    # Print results
    print(message)
    print(f"User: {user}")
    print(f"Role: {user.role.value}")
    print(f"Database: {db.connection_string}")
    print(f"Total: {total}")

    return {"user": user.name, "email": user.email, "role": user.role.value, "total": total, "message": message}


if __name__ == "__main__":
    result = main()
    print("Result:", result)
