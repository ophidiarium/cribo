"""Main module demonstrating importlib usage with static string literals."""

import importlib

# Import modules with hyphens in their names
# Can't do: import data-processor  (SyntaxError)
data_processor = importlib.import_module("data-processor")
api_client = importlib.import_module("api-client")

# Import modules with Python keywords as names
# Can't do: import class  (SyntaxError: invalid syntax)
class_module = importlib.import_module("class")
def_module = importlib.import_module("def")
for_module = importlib.import_module("for")

# Import module starting with number
# Can't do: import 2024-config  (SyntaxError)
config_2024 = importlib.import_module("2024-config")


def main():
    # Test hyphenated modules
    processor = data_processor.DataProcessor()
    print(f"Processor: {processor.name}")
    print(f"Processing result: {processor.process('test data')}")
    print(f"Processor info: {data_processor.get_processor_info()}")
    print(f"Processor version: {data_processor.PROCESSOR_VERSION}")
    print()

    client = api_client.create_client()
    print(f"API request: {client.make_request('users')}")
    print(f"API version: {api_client.API_VERSION}")
    print(f"Supported endpoints: {api_client.SUPPORTED_ENDPOINTS}")
    print()

    # Test keyword-named modules
    meta = class_module.create_class_instance("TestClass")
    print(f"Class description: {meta.describe()}")
    print(f"Class types: {class_module.CLASS_TYPES}")
    print(f"Default class name: {class_module.DEFAULT_CLASS_NAME}")
    print()

    func_def = def_module.define_function("calculate", "x", "y")
    print(f"Function definition: {func_def}")
    print(f"Builtin functions: {def_module.BUILTIN_FUNCTIONS}")
    print()

    loop = for_module.create_loop(["apple", "banana", "cherry"])
    print(f"Loop results: {loop.iterate()}")
    print(f"Loop types: {for_module.LOOP_TYPES}")
    print(f"Max iterations: {for_module.MAX_ITERATIONS}")
    print()

    # Test module starting with number
    config = config_2024.load_yearly_config()
    print(f"Year config: {config.get_config()}")
    print(f"Supported years: {config_2024.SUPPORTED_YEARS}")
    print(f"Config prefix: {config_2024.CONFIG_PREFIX}")


if __name__ == "__main__":
    main()
