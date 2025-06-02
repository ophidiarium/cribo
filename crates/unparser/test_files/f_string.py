world = "World"
empty_f_string = f""  # noqa: F541
f"{world!a:}None{empty_f_string!s:}None"
answer = 42.000001
f"{answer:.03f}"
f"'\"'''\"\"\"{{}}\\"  # noqa
if __name__ == "__main__":
    print(f"Hello {world}!")

# fmt: off
lines = "\n".join(
            f'''<clipPath id="{unique_id}-line-{line_no}">{make_tag({"rect"}, x=0, y=offset, width=char_width * width, height=line_height + 0.25)}</clipPath>''' # noqa
            for line_no, offset in enumerate(line_offsets) # noqa
        )

tag_attribs = " ".join(
    (
        f'''{k.lstrip('_').replace('_', '-')}="{stringify(v)}"''' # noqa
        for (k, v) in attribs.items() # noqa
    )
)

completion_init_lines = [f"source '{completion_path}'"] # noqa

# fmt: on
