#!/usr/bin/env python3
"""Simple test project entry point."""

from utils.helpers import greet, calculate
from models.user import User

def main():
    """Main function."""
    user = User("Alice", 30)
    print(greet(user.name))
    
    result = calculate(10, 20)
    print(f"Calculation result: {result}")
    
    print(f"User: {user.name}, Age: {user.age}")

if __name__ == "__main__":
    main()
