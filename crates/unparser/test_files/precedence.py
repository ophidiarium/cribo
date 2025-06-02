import dataclasses

reduce = "abcdefhijklmnopqrstuvwxyz"
reduce_list = (list(reduce) + [None] * 5)[:5]


@dataclasses.dataclass
class MyDataclass:
    name: str


# from pydantic._internal._fields
dataclass_fields = {field.name for field in (dataclasses.fields(MyDataclass) if dataclasses.is_dataclass(MyDataclass) else ())}

a, b, c = "a", "b", "c"
