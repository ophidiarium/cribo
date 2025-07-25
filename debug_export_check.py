"""Check if __version__ should be exported"""

# __version__ doesn't start with underscore (except the dunder)
# So it should be exported according to Python's default visibility rules
print(f"__version__ starts with underscore: {'__version__'.startswith('_')}")
print(f"__version__ is __all__: {'__version__' == '__all__'}")
