"""Package A processor that imports utils expecting package_a.utils"""

# This is the key - importing just "utils" when inside package_a
import utils


def process_a(data):
    return utils.transform(data)
