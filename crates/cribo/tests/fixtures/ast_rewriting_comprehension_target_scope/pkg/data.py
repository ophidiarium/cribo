# Comprehension target "x" shadows module-level "x".
# The symbol dependency graph must NOT record "items" as depending on "x".
items = [x for x in range(5)]
from pkg.helper import transform

x = len(items)
result = transform(x)
