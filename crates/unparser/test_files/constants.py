escaped_string_expr = "'\"'''\"\"\"{}\\"
escaped_unicode_expr = u"'\"'''\"\"\"{}\\"  # fmt: skip
escaped_bytes_expr = b"'\"'''\"\"\"{}\\"


# real code from yaml.constructor
# https://github.com/yaml/pyyaml/blob/69c141adcf805c5ebdc9ba519927642ee5c7f639/lib/yaml/constructor.py#L265
inf_value = 1e300

# if inf_value would not be initialized in scientific notation
# the following loop would run for a long time
while inf_value != inf_value * inf_value:
    inf_value *= inf_value

scientific_fraction = 1e-69

complex_number = 3 + 4j
