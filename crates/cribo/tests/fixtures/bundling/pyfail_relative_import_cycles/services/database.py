from .auth import get_user_context


def get_connection():
    """Get database connection with user context"""
    context = get_user_context()
    return f"db_connection(context={context})"


def execute_query(query):
    """Execute a database query"""
    return f"query_result({query})"
