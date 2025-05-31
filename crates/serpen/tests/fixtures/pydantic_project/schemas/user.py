"""User schema definitions using Pydantic."""

from pydantic import BaseModel, EmailStr, Field
from typing import Optional

class UserSchema(BaseModel):
    """User model with Pydantic validation."""
    
    name: str = Field(..., min_length=1, max_length=100, description="User's full name")
    email: EmailStr = Field(..., description="User's email address")
    age: int = Field(..., ge=0, le=150, description="User's age")
    is_active: bool = Field(default=True, description="Whether the user is active")
    bio: Optional[str] = Field(default=None, max_length=500, description="User's biography")
    
    class Config:
        """Pydantic configuration."""
        json_encoders = {
            # Custom encoders if needed
        }
        schema_extra = {
            "example": {
                "name": "John Doe",
                "email": "john@example.com",
                "age": 30,
                "is_active": True,
                "bio": "Software developer"
            }
        }

class CreateUserRequest(BaseModel):
    """Request model for creating a new user."""
    
    name: str = Field(..., min_length=1, max_length=100)
    email: EmailStr
    age: Optional[int] = Field(default=None, ge=0, le=150)
    bio: Optional[str] = Field(default=None, max_length=500)

class UserResponse(BaseModel):
    """Response model for user data."""
    
    id: int
    name: str
    email: str
    age: int
    is_active: bool
    bio: Optional[str] = None
