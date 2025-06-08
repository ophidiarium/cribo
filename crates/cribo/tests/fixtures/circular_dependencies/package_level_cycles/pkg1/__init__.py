from pkg2 import helper_function


def main_function():
    """Main function that uses helper from pkg2"""
    return f"pkg1.main({helper_function()})"


def utility_function():
    """Utility that pkg2 will import"""
    return "pkg1_utility"
