simple_assign = str()
x1, x2 = (1, 2)
first = second = "third"
name_1, name_4, name_2, name_4, name_3 = name_2 = [name_0] = name_0.name_4 = (
    name_5.name_2  # type: ignore[name-defined, has-type] # noqa
) = () = {name_5: name_1, name_0: name_3}  # type: ignore[name-defined, has-type] # noqa
