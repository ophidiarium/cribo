# Import Patterns Summary for Test Fixtures

This document lists every fixture under `crates/cribo/tests/fixtures` and the specific import patterns each exercises.

## All Fixtures

- alias_transformation_test
- all_variable_handling
- ast_rewriting_class_name_collision
- ast_rewriting_function_name_collision
- ast_rewriting_variable_name_collision
- ast_rewriting_happy_path
- ast_rewriting_regular_import_aliases
- comprehensive_ast_rewrite
- fstring_globals_lifting
- fstring_module_globals
- function_level_module_import
- future_imports_basic
- future_imports_multiple
- init_reexports
- mixed_import_patterns
- namespace_simple_test
- pandera-polars
- pydantic_project
- pyfail_class_level_cycles
- pyfail_four_module_cycle
- pyfail_mixed_cycles
- pyfail_package_level_cycles
- pyfail_relative_import_cycles
- pyfail_three_module_cycle
- pyfail_unresolvable_patterns
- simple_math
- simple_project
- test_all_exports
- stickytape_circular_reference
- stickytape_explicit_relative_import
- stickytape_explicit_relative_import_from_parent_package
- stickytape_explicit_relative_import_in_init
- stickytape_explicit_relative_import_single_dot
- stickytape_explicit_relative_import_single_dot_in_init
- stickytape_implicit_init_import
- stickytape_import_from_as_module
- stickytape_import_from_as_value
- stickytape_imports_in_imported_modules
- stickytape_module_with_triple_quotes
- stickytape_script_using_from_to_import_module
- stickytape_script_using_from_to_import_multiple_values
- stickytape_script_using_module_in_package
- stickytape_script_using_stdlib_module_in_package
- stickytape_script_with_single_local_from_import
- stickytape_script_with_single_local_import
- stickytape_script_with_single_local_import_of_package
- stickytape_script_with_special_shebang
- stickytape_single_file
- stickytape_single_file_using_stdlib
- xfail_stickytape_explicit_relative_import_single_dot
- xfail_stickytape_script_using_from_to_import_module
- xfail_stickytape_script_with_dynamic_import

## Patterns Grouped by Category

### 1. alias_transformation_test

- import json as j
- import os as operating_system
- import sys as system_info
- from utils.data_processor import process_data as process_a
- from utils.data_processor import format_output as format_a
- from utils.config_manager import load_config as config_a
- from utils.helpers import helper_func, debug_print as debug_a

### 2. all_variable_handling

- from simple_module import public_func, CONSTANT
- from nested_package import exported_from_init
- from nested_package.submodule import sub_function
- import simple_module
- import nested_package.submodule as sub
- from conflict_module import message

### 3. ast_rewriting_class_name_collision

- from models import User as ModelUser, Product as ModelProduct
- from entities import User as EntityUser, Product as EntityProduct

### 4. ast_rewriting_function_name_collision

- from module_a import process_data as process_a
- from module_b import process_data as process_b

### 5. ast_rewriting_variable_name_collision

- from constants import API_URL, VERSION, DEFAULT_TIMEOUT
- from config import API_URL as CONFIG_API_URL, VERSION as CONFIG_VERSION, MAX_RETRIES

### 6. ast_rewriting_happy_path

- from utils.helpers import format_message, calculate_total
- from models.user import User, UserRole
- from services.database import DatabaseService

### 7. ast_rewriting_regular_import_aliases

- import os as operating_system
- import json as j
- import sys as system_module
- import collections.abc as abc_collections
- import urllib.parse as url_parser
- import xml.etree.ElementTree as xml_tree
- import utils.helpers as helper_utils
- import utils.config as config_module
- import math
- import random
- import datetime

### 8. comprehensive_ast_rewrite

- from core.database.connection import process as db_process
- from core.utils.helpers import process, Logger as UtilLogger, validate
- from services.auth.manager import process as auth_process, User, validate as auth_validate
- from models.user import User as UserModel, process_user, Logger
- from models import base

### 9. fstring_globals_lifting

- f-strings referencing module-level globals (lifted by transformer)

### 10. fstring_module_globals

- from worker import Worker
- f-strings referencing Worker methods/globals

### 11. function_level_module_import

- from utils import calculator (inside process_data())

### 12. future_imports_basic

- from **future** import annotations
- from mypackage.core import process_data
- from mypackage.submodule.utils import validate_input

### 13. future_imports_multiple

- from **future** import annotations, print_function
- from module_a import func_a
- from module_b import func_b

### 14. init_reexports

- from mypackage import format_data, process_data, config
- from mypackage.utils import helper_function

### 15. mixed_import_patterns

- import app
- from config import get_config
- from logger import get_logger
- from utils import format_message
- alias imports in submodules

### 16. namespace_simple_test

- import mymodule.utils
- from mymodule.utils import helper

### 17. pandera-polars

- import pandera.polars as pa
- import polars as pl
- from pandera.typing import DataFrame
- from schema import CitySchema

### 18. pydantic_project

- import json
- from pydantic import ValidationError
- from schemas.user import UserSchema, CreateUserRequest
- from utils.validation import validate_email

### 19. pyfail_class_level_cycles

- from user_class import User
- from admin_class import Admin
- mutual class-level imports triggering cycle

### 20. pyfail_four_module_cycle

- import module_a
- from module_b import process_in_b
- from module_c import process_in_c
- from module_d import process_in_d
- from module_a import final_step

### 21. pyfail_mixed_cycles

- from constants_module import BASE_VALUE
- from function_module import process_data
- from config_constants import CONFIG_MULTIPLIER

### 22. pyfail_package_level_cycles

- import pkg1

### 23. pyfail_relative_import_cycles

- from services import auth (relative import cycle)

### 24. pyfail_three_module_cycle

- import module_a
- from module_b import process_b
- from module_c import process_c
- from module_a import get_value_a

### 25. pyfail_unresolvable_patterns

- import constants_a
- import constants_b
- from constants_a import A_VALUE
- from constants_b import B_VALUE

### 26. simple_math

- from calculator import add, multiply
- from utils import format_result

### 27. simple_project

- from utils.helpers import greet, calculate
- from models.user import User

### 28. test_all_exports

- from utils import helper_function, UtilityClass

### 29. stickytape_circular_reference

- import first
- import second

### 30. stickytape_explicit_relative_import

- import greetings.greeting

### 31. stickytape_explicit_relative_import_from_parent_package

- import greetings.greeting

### 32. stickytape_explicit_relative_import_in_init

- import greetings

### 33. stickytape_explicit_relative_import_single_dot

- import greetings.greeting

### 34. stickytape_explicit_relative_import_single_dot_in_init

- import greetings

### 35. stickytape_implicit_init_import

- import greetings.irrelevant

### 36. stickytape_import_from_as_module

- from greetings import greeting as g

### 37. stickytape_import_from_as_value

- from greeting import message as m

### 38. stickytape_imports_in_imported_modules

- from greetings import message

### 39. stickytape_module_with_triple_quotes

- import greeting

### 40. stickytape_script_using_from_to_import_module

- from greetings import greeting

### 41. stickytape_script_using_from_to_import_multiple_values

- from greeting import print_stdout, message

### 42. stickytape_script_using_module_in_package

- import greetings.greeting

### 43. stickytape_script_using_stdlib_module_in_package

- import xml.etree.ElementTree
- import greeting

### 44. stickytape_script_with_single_local_from_import

- from greeting import message

### 45. stickytape_script_with_single_local_import

- import greeting

### 46. stickytape_script_with_single_local_import_of_package

- import greeting

### 47. stickytape_script_with_special_shebang

- import sys

### 48. stickytape_single_file

- no imports (single-file script)

### 49. stickytape_single_file_using_stdlib

- import hashlib

### 50. xfail_stickytape_explicit_relative_import_single_dot

- import greetings.greeting (xfail)

### 51. xfail_stickytape_script_using_from_to_import_module

- from greetings import greeting (xfail)

### 52. xfail_stickytape_script_with_dynamic_import

- import importlib (xfail)
