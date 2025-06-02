def name_4[*name_2](  # type: ignore
    name_2,
    /,
    name_4,
    name_1,
    *name_0: str,
    name_3: str = "",
    name_5: str = "",
) -> str:
    return ""


async def name_5[*name_2](  # type: ignore
    name_2,
    /,
    name_4,
    name_1,
    *name_0: str,
    name_3: str = "",
    name_5: str = "",
) -> str:
    return ""


def type_comment() -> str:  # type: ignore[misc]
    return ""


async def type_comment_async():  # type: ignore[misc]
    return ""


def name_8(
    name_5: name_4,  # type: ignore
    name_3,
    name_4,
    /,
    *,
    name_1=name_2 <= name_1,  # type: ignore # noqa
    name_0=~name_5,  # type: ignore
):
    pass


async def name_5[**name_5, *name_3, **name_2](  # type: ignore
    name_1: name_1,  # type: ignore # noqa
    name_2,
    name_0: name_2,  # type: ignore
    /,  # type: ignore
) -> name_3:  # type: ignore
    name_1
