from . import config

message = "Hello"


def get_default_greeting():
    return f"{message}, {config.DEFAULT_NAME}!"
