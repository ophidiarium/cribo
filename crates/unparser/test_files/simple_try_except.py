# example comes from fuzzing

try:
    name_0  # type: ignore # noqa: F821
    name_4  # type: ignore # noqa: F821
    name_4  # type: ignore # noqa: F821
    name_1  # type: ignore # noqa: F821
except name_1:  # type: ignore # noqa: F821
    pass
except name_4 as name_5:  # type: ignore # noqa: F821, F841
    pass
    pass
except:  # noqa: E722
    pass
    pass
else:
    pass
    pass
finally:
    pass
    pass
    pass
    pass
    pass
