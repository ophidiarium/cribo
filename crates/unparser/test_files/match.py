name_2 = 2
name_5 = 5


class MatchThis:
    field_1 = "Test1"
    field_2 = "Test2"


match {name_5: name_2}:
    case None:
        pass
    case [value_1, *rest] if []:
        print(value_1, *rest)
    case (value_1, *rest) if []:
        print(value_1, *rest)
    case MatchThis(field_1=field1_value, field_2=field2_value):
        print(field1_value, field2_value)
    case {"field_1": field1_value, "field_2": field2_value, **kwargs}:
        print(field1_value, field2_value, **kwargs)  # type: ignore
    case _ if lambda *name_4, **name_2: name_5:  # type: ignore
        pass
