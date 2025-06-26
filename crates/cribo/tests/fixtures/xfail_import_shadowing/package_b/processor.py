"""Package B processor that imports utils expecting package_b.utils"""

# This is the key - importing just "utils" when inside package_b
import utils


def process_b(data):
    return utils.transform(data)
