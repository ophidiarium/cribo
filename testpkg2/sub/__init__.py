
print("Initializing testpkg2.sub")

def check_parent_access():
    # Try to access parent module - will this work?
    try:
        print(f"Can access testpkg2? {testpkg2}")
        return True
    except NameError as e:
        print(f"Cannot access testpkg2: {e}")
        return False
