# Test case for __all__ variable being referenced in a loop
# This pattern is used by httpx to set __module__ attributes

from mypkg import MyClass, my_func

# Should work correctly
obj = MyClass()
print(obj.value)
print(my_func())
