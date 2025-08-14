"""Test that demonstrates metaclass ordering issue with name collisions."""

# Import from both modules to trigger potential name collision handling
from module_a import YAMLObject
from module_b import YAMLObject as YAMLObjectB


def test_both_classes():
    """Test that both metaclass-based classes work."""

    # Test first class
    class MyObjectA(YAMLObject):
        yaml_tag = "!myobject_a"

        def __init__(self, value):
            self.value = value

    obj_a = MyObjectA(42)
    assert obj_a.value == 42
    assert MyObjectA.yaml_tag == "!myobject_a"

    # Test second class
    class MyObjectB(YAMLObjectB):
        yaml_tag = "!myobject_b"

        def __init__(self, value):
            self.value = value * 2

    obj_b = MyObjectB(21)
    assert obj_b.value == 42
    assert MyObjectB.yaml_tag == "!myobject_b"

    print("Both metaclass tests passed")


if __name__ == "__main__":
    test_both_classes()
