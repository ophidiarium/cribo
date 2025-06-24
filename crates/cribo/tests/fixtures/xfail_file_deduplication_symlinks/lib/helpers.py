# lib/helpers.py


def get_location():
    return "lib/helpers.py"


counter = 0


def increment_counter():
    global counter
    counter += 1
    return counter
