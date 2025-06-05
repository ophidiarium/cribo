from .database import get_connection


def authenticate_user(username):
    """Authenticate user using database connection"""
    conn = get_connection()
    return f"auth({username}, conn={conn})"


def get_user_context():
    """Get user context - used by database module"""
    return "user_context_from_auth"
