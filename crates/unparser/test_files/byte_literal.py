byte_literal = b"hello"

# extracted from pygments.lexer
_encoding_map = [
    (b"\xef\xbb\xbf", "utf-8"),
    (b"\xff\xfe\0\0", "utf-32"),
    (b"\0\0\xfe\xff", "utf-32be"),
    (b"\xff\xfe", "utf-16"),
    (b"\xfe\xff", "utf-16be"),
]
