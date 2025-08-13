from mypackage import abc

# Use the imported module and demonstrate that ABC is accessible
obj = abc.MyClass()
print(obj.get_name())
print(f"Is MyClass a subclass of ABC? {issubclass(abc.MyClass, abc.ABC)}")
