"""
Base model functionality with relative import conflicts
"""

from typing import Any, Dict
from ..core.utils.helpers import validate as core_validate  # Cross-package relative
from .user import Logger as UserLogger  # Same-package relative import

# Global conflicts
result = "base_result"
process = "base_process_string"


class BaseModel:
    """Base model class with method name conflicts"""

    def __init__(self, model_type: str):
        self.model_type = model_type
        self.initialized = False
        # Instance variables with conflicted names
        self.validate = self._base_validate
        self.process = self._base_process
        self.Logger = UserLogger("base_model")

    def _base_validate(self, data: Any) -> bool:
        """Base validation using relative imports"""
        return core_validate(data)  # Using cross-package import

    def _base_process(self, data: Any) -> str:
        """Base processing with conflicts"""
        validated = self.validate(data)
        if validated:
            self.Logger._log_process(f"Base processing: {data}")
            return f"base_processed: {data}"
        return "base_invalid"

    def initialize(self) -> str:
        """Initialize with name conflicts"""
        global result
        self.initialized = True
        result = f"base_initialized_{self.model_type}"
        return result


def initialize() -> str:
    """Module initialization function"""
    global result

    # Create base model with conflicts
    base = BaseModel("default")
    init_result = base.initialize()

    # Use relative imports with conflicts
    logger = UserLogger("base_init")
    logger._log_process("Base module initialized")

    result = f"module_init: {init_result}"
    return result


def validate(data: Any) -> bool:
    """Base validate function - conflicts everywhere"""
    return core_validate(data) and data != "invalid"


def process(data: Any) -> str:
    """Base process function - conflicts everywhere"""
    global result

    validated = validate(data)
    if validated:
        processed = f"base_module_process: {data}"
    else:
        processed = "base_module_invalid"

    result = f"base_last_process: {processed}"
    return processed


class Logger:
    """Base Logger class - yet another Logger conflict"""

    def __init__(self, source: str):
        self.source = source
        self.logs = []

    def log(self, message: str) -> None:
        self.logs.append(f"[BASE {self.source}] {message}")

    def process(self, log_data: Any) -> str:
        """Logger process method - conflicts with global process"""
        self.log(f"Processing: {log_data}")
        return f"base_logger_process: {log_data}"


def connect() -> str:
    """Base connect function"""
    return "base_connected"


# Function with parameter shadowing all conflict names
def shadow_test(validate: Any = None, process: Any = None, Logger: Any = None, result: Any = None, initialize: Any = None) -> Dict[str, Any]:
    """Function that shadows all major conflict names with parameters"""

    # Parameters shadow all the global/class names
    shadows = {"validate_param": validate, "process_param": process, "Logger_param": Logger, "result_param": result, "initialize_param": initialize}

    # Local variables that shadow parameters and globals
    validate = globals()["validate"]  # Get global function
    process = globals()["process"]  # Get global function
    Logger = globals()["Logger"]  # Get global class

    # Use the retrieved globals
    validation_result = validate("test_data")
    process_result = process("test_data")
    logger = Logger("shadow_test")

    shadows.update({"global_validate_result": validation_result, "global_process_result": process_result, "global_logger_source": logger.source})

    return shadows


# More global assignments for conflict testing
validate = validate
process = process
Logger = Logger
initialize = initialize
