#!/usr/bin/env python3
"""Pydantic test project entry point."""

import json
from pydantic import ValidationError

from schemas.user import UserSchema, CreateUserRequest
from utils.validation import validate_email

def main():
    """Main function demonstrating Pydantic usage."""
    # Create a valid user
    user_data = {
        "name": "John Doe",
        "email": "john@example.com",
        "age": 25
    }
    
    try:
        user = UserSchema(**user_data)
        print(f"Created user: {user}")
        
        # Test validation
        if validate_email(user.email):
            print("Email validation passed")
        
        # Test serialization
        user_json = user.model_dump_json()
        print(f"User JSON: {user_json}")
        
        # Test create request
        request = CreateUserRequest(name="Jane Doe", email="jane@example.com")
        print(f"Create request: {request}")
        
    except ValidationError as e:
        print(f"Validation error: {e}")

if __name__ == "__main__":
    main()
