"""Unit tests for the cribo source map runtime internals.

Driven by the Rust integration test `python_runtime_unit_tests` (plain asserts,
no pytest dependency). Usage: python test_sourcemap_runtime.py <runtime.py>
where <runtime.py> is the template with the mode placeholder substituted.

Tests are discovered automatically: every module-level callable whose name
starts with ``test_`` runs once, receiving the runtime instance.
"""

import importlib.util
import os
import sys
import tempfile
import threading


def load_runtime(path):
    """Import the runtime module and return a runtime instance for testing.

    The import installs the hooks; they are restored immediately so failures
    in this harness surface as normal tracebacks. The runtime class is
    recovered from the installed hook's bound instance — the template's last
    statement deletes every module-level helper name, so the module namespace
    is intentionally empty after import.
    """
    prev_hooks = (sys.excepthook, sys.unraisablehook, threading.excepthook)
    spec = importlib.util.spec_from_file_location("cribo_sm_runtime", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    installed_hook = sys.excepthook
    sys.excepthook, sys.unraisablehook, threading.excepthook = prev_hooks
    assert not hasattr(module, "_CriboSourceMapRuntime"), (
        "template must scrub its helper names from the bundle namespace"
    )
    runtime_cls = type(installed_hook.__self__)
    return runtime_cls(
        "external",
        "<test-bundle>",
        "",  # no build digest: verification is skipped
        os,
        __import__("binascii"),
        threading,
        __import__("traceback"),
        __import__("hashlib"),
    )


def test_stream_reads_across_chunk_boundaries(rt):
    stream = rt._stream_cls([b"ab", b"", b"c", b"de"])
    got = []
    while True:
        byte = stream.read_byte()
        if byte < 0:
            break
        got.append(chr(byte))
    assert got == list("abcde"), got
    assert stream.read_byte() == -1  # stays exhausted


def _scan(rt, json_text, needed, max_needed):
    return rt._scan(lambda: [json_text.encode("utf-8")], needed, max_needed)


def test_scan_extracts_sources_and_mappings(rt):
    # AAAA;AACA;AACA: one segment per line, source line advancing by one.
    json_text = '{"version":3,"sources":["a.py","b.py"],"mappings":"AAAA;AACA;AACA"}'
    sources, names, table = _scan(rt, json_text, {0, 2}, 2)
    assert sources == {0: "a.py"}, sources  # only referenced indices collected
    assert names == {}, names
    assert table == {0: (0, 0, None), 2: (0, 2, None)}, table


def test_scan_negative_delta(rt):
    # Line 0 advances src_line by +1 (C), line 1 rewinds it by -1 (D).
    json_text = '{"sources":["a.py"],"mappings":"AACA;AADA"}'
    _sources, _names, table = _scan(rt, json_text, {0, 1}, 1)
    assert table == {0: (0, 1, None), 1: (0, 0, None)}, table


def test_scan_source_index_delta(rt):
    # Second line switches to source 1 (C in field 1).
    json_text = '{"sources":["a.py","b.py"],"mappings":"AAAA;ACAA"}'
    _sources, _names, table = _scan(rt, json_text, {0, 1}, 1)
    assert table == {0: (0, 0, None), 1: (1, 0, None)}, table


def test_scan_extracts_original_names(rt):
    json_text = '{"sources":["a.py"],"names":["original"],"mappings":"AAAAA"}'
    sources, names, table = _scan(rt, json_text, {0}, 0)
    assert sources == {0: "a.py"}, sources
    assert names == {0: "original"}, names
    assert table == {0: (0, 0, 0)}, table


def test_scan_ignores_adversarial_sources_content(rt):
    # sourcesContent value contains fake keys and escaped quotes; the string
    # lexer must skip it without being fooled.
    evil = '\\"mappings\\": \\"ZZZZ\\", \\\\'
    json_text = (
        '{"sources":["a.py"],"sourcesContent":["' + evil + '"],"mappings":"AAAA"}'
    )
    sources, _names, table = _scan(rt, json_text, {0}, 0)
    assert sources == {0: "a.py"}, sources
    assert table == {0: (0, 0, None)}, table


def test_scan_handles_unicode_escapes_in_sources(rt):
    json_text = '{"sources":["\\u00e9t\\u00e9.py"],"mappings":"AAAA"}'
    sources, _names, _table = _scan(rt, json_text, {0}, 0)
    assert sources == {0: "\u00e9t\u00e9.py"}, sources


def test_scan_null_in_sources_array(rt):
    json_text = '{"sources":["a.py",null,"c.py"],"mappings":"ACAA"}'
    sources, _names, _table = _scan(rt, json_text, {0}, 0)
    assert sources == {}, sources  # a null entry is simply not collected


def test_vlq_early_exit_stops_reading(rt):
    # The decoder must stop pulling chunks once past max_needed: the second
    # chunk raises if consumed.
    class Boom(Exception):
        pass

    def chunks_factory():
        yield b'{"sources":["a.py"],"mappings":"AAAA;AACA;'
        raise Boom("decoder read past its early-exit point")

    _sources, _names, table = rt._scan(chunks_factory, {0}, 0)
    assert table == {0: (0, 0, None)}, table


def test_vlq_rejects_escapes_in_mappings(rt):
    json_text = '{"sources":["a.py"],"mappings":"AA\\\\AA"}'
    try:
        _scan(rt, json_text, {0}, 0)
    except ValueError:
        pass
    else:
        raise AssertionError("escape inside mappings must raise")


def test_scan_handles_mappings_before_sources(rt):
    # A spec-valid map may order keys arbitrarily. Early exit inside the
    # mappings string must not derail parsing of a later sources field.
    json_text = '{"mappings":"AAAA;AACA;AACA","sources":["a.py","b.py"]}'
    sources, _names, table = _scan(rt, json_text, {0}, 0)  # early exit after line 0
    assert sources == {0: "a.py"}, sources
    assert table == {0: (0, 0, None)}, table


def test_scan_combines_surrogate_pairs(rt):
    # Non-BMP characters arrive as JSON surrogate pairs; they must decode to
    # one code point, not two replacement characters.
    json_text = '{"sources":["\\ud83d\\ude00.py"],"mappings":"AAAA"}'
    sources, _names, _table = _scan(rt, json_text, {0}, 0)
    assert sources == {0: "\U0001f600.py"}, sources
    # A lone high surrogate followed by a plain character stays recoverable
    # (replacement character), and the rest of the string is intact.
    json_text = '{"sources":["\\ud83dx.py"],"mappings":"AAAA"}'
    sources, _names, _table = _scan(rt, json_text, {0}, 0)
    assert sources[0].endswith("x.py"), sources


def make_inline_bundle(payload_json):
    """Create a temp file shaped like an inline-mode bundle; return its path."""
    import base64

    encoded = base64.b64encode(payload_json.encode("utf-8")).decode("ascii")
    handle = tempfile.NamedTemporaryFile(
        "w", suffix=".py", delete=False, encoding="utf-8"
    )
    with handle as f:
        f.write("print('hello')\n" * 300)  # push the marker past one chunk
        f.write("# sourceMappingURL=data:application/json;base64," + encoded + "\n")
    return handle.name


def test_inline_payload_scan_and_chunked_base64(rt):
    # Payload much larger than one 8 KiB chunk exercises 4-byte alignment
    # handling across chunk boundaries.
    filler = "x" * 40000
    json_text = '{"filler":"' + filler + '","sources":["a.py"],"mappings":"AAAA"}'
    path = make_inline_bundle(json_text)
    try:
        with open(path, "rb") as handle:
            decoded = b"".join(rt._inline_chunks(handle))
        assert decoded.decode("utf-8") == json_text
        # And end-to-end through the scanner (one pinned handle, two passes):
        with open(path, "rb") as handle:
            sources, _names, table = rt._scan(lambda: rt._inline_chunks(handle), {0}, 0)
        assert sources == {0: "a.py"}, sources
        assert table == {0: (0, 0, None)}, table
    finally:
        os.unlink(path)


def test_inline_scan_without_marker_yields_nothing(rt):
    handle = tempfile.NamedTemporaryFile(
        "w", suffix=".py", delete=False, encoding="utf-8"
    )
    with handle as f:
        f.write("print('no map here')\n" * 50)
    try:
        with open(handle.name, "rb") as bundle:
            assert b"".join(rt._inline_chunks(bundle)) == b""
    finally:
        os.unlink(handle.name)


def test_json_fallback_matches_streaming(rt):
    json_text = '{"sources":["a.py","b.py"],"mappings":"AAAA;ACCA"}'
    streaming = _scan(rt, json_text, {0, 1}, 1)

    handle = tempfile.NamedTemporaryFile(
        "w", suffix=".map", delete=False, encoding="utf-8"
    )
    with handle as f:
        f.write(json_text)
    path = handle.name
    previous = os.environ.get("CRIBO_SOURCE_MAPS")
    try:
        os.environ["CRIBO_SOURCE_MAPS"] = path
        loaded = rt._load_json_fallback({1, 2})
        assert loaded is not None
        table, sources, names, _map_dir = loaded
        # Fallback tables are 1-based.
        expected = {
            line0 + 1: (idx, src_line0 + 1, name_idx)
            for line0, (idx, src_line0, name_idx) in streaming[2].items()
        }
        assert table == expected, (table, expected)
        assert sources == streaming[0]
        assert names == streaming[1]
    finally:
        if previous is None:
            os.environ.pop("CRIBO_SOURCE_MAPS", None)
        else:
            os.environ["CRIBO_SOURCE_MAPS"] = previous
        os.unlink(path)


def test_env_path_wins_for_every_mode(rt):
    # A CRIBO_SOURCE_MAPS path activates the runtime even for a <stdin> bundle
    # (the stdin piping workflow cannot re-read its own inline map).
    handle = tempfile.NamedTemporaryFile(
        "w", suffix=".map", delete=False, encoding="utf-8"
    )
    with handle as f:
        f.write('{"sources":["a.py"],"mappings":"AAAA"}')
    path = handle.name
    previous = os.environ.get("CRIBO_SOURCE_MAPS")
    try:
        os.environ["CRIBO_SOURCE_MAPS"] = path
        inline_stdin = type(rt)(
            "inline",
            "<stdin>",
            "",
            os,
            __import__("binascii"),
            threading,
            __import__("traceback"),
            __import__("hashlib"),
        )
        loaded = inline_stdin._load({1})
        assert loaded is not None, "env path must activate a <stdin> inline bundle"
        table, sources, names, _map_dir = loaded
        assert sources == {0: "a.py"}, sources
        assert names == {}, names
        assert table == {1: (0, 1, None)}, table
        # Without the env override, a <stdin> inline bundle stays inactive.
        os.environ.pop("CRIBO_SOURCE_MAPS", None)
        assert inline_stdin._map_location() is None
    finally:
        if previous is None:
            os.environ.pop("CRIBO_SOURCE_MAPS", None)
        else:
            os.environ["CRIBO_SOURCE_MAPS"] = previous
        os.unlink(path)


def test_digest_verification_errors_fail_closed(rt):
    class BrokenHashlib:
        @staticmethod
        def sha256():
            raise RuntimeError("hashing unavailable")

    with tempfile.TemporaryDirectory() as directory:
        bundle = os.path.join(directory, "bundle.py")
        with open(bundle, "w", encoding="utf-8") as handle:
            handle.write("raise RuntimeError('boom')\n")
        with open(bundle + ".map", "w", encoding="utf-8") as handle:
            handle.write('{"sources":["a.py"],"mappings":"AAAA"}')

        previous = (
            rt._mode,
            rt._bundle_anchor,
            rt._expected_digest,
            rt._hashlib,
        )
        try:
            rt._mode = "linked"
            rt._bundle_anchor = bundle
            rt._expected_digest = "0" * 64
            rt._hashlib = BrokenHashlib()
            assert rt._load({1}) is None
            assert rt._load_json_fallback({1}) is None
        finally:
            (
                rt._mode,
                rt._bundle_anchor,
                rt._expected_digest,
                rt._hashlib,
            ) = previous


def test_percent_decode_restores_non_utf8_unix_paths(rt):
    if os.name != "posix":
        return
    decoded = rt._percent_decode("odd%FF/name.py")
    assert os.fsencode(decoded) == b"odd\xff/name.py"


def main():
    runtime_path = sys.argv[1]
    rt = load_runtime(runtime_path)
    tests = sorted(
        (
            obj
            for name, obj in globals().items()
            if name.startswith("test_") and callable(obj)
        ),
        key=lambda obj: obj.__name__,
    )
    for test in tests:
        test(rt)
        print("PASS %s" % test.__name__)
    print("ALL %d RUNTIME TESTS PASSED" % len(tests))


if __name__ == "__main__":
    main()
