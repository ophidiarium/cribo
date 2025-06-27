"""Test nested namespace imports"""

import greetings.greeting

# Use the nested module
message = greetings.greeting.get_greeting("Python")
print(message)
