import random
import string

generator = (value for value in random.choices(string.ascii_letters))
set_ = {def_ for def_ in random.choices(string.ascii_letters)}
dict_ = {k: v for k, v in enumerate(random.choices(string.ascii_letters))}
list_ = [value for value in random.choices(string.ascii_letters)]


with_outer_if = (value for value in random.choices(string.ascii_letters) if value)
with_outer_if_not = (value for value in random.choices(string.ascii_letters) if not value if value != value)
with_inner_if_else = (value if value else "missing!" for value in random.choices(string.ascii_letters))
