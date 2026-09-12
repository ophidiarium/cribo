"""Cribo source map runtime (injected prologue).

Remaps tracebacks of uncaught exceptions back to the original source files
using the Source Map v3 emitted at bundle time. Lazy by design: no file I/O,
parsing, or decoding happens at import time; everything is deferred to the
first uncaught exception. Under resource pressure the decoder streams the map
in constant memory and falls back to the default traceback on any failure.
See docs/source-maps.md in the cribo repository.

Note: this leading docstring is stripped at injection time so the bundle's
``__doc__`` is not affected.
"""

import sys as _cribo_sys


def _cribo_sm_import(
    name,
    script_path=None,
    *,
    _sys=_cribo_sys,
    _list=list,
    _import=__import__,
    _isinstance=isinstance,
    _str=str,
    _bex=BaseException,
):
    """Import a stdlib module immune to script-directory shadowing.

    The bundle's directory can reach `sys.path` several ways: as `sys.path[0]`
    (direct execution), via `PYTHONPATH`, or not at all under `runpy` (where
    `sys.path[0]` is the *driver's* directory) — so the bundle's own path is
    passed in explicitly and every equivalent entry is removed. The complete
    resolution/execution window holds the interpreter import lock; while the
    resolved stdlib module executes, `sys.path` is temporarily the filtered
    snapshot so its *transitive* imports cannot select adjacent project files.
    Other imports block on the same lock and never observe that temporary path.
    Lexical de-duplication works even before `os` is importable (`python -S`);
    once `os` is available, entries are also compared by absolute path.
    Non-string entries are kept untouched. (`sys`, `_imp`, and the frozen
    importlib modules are builtins and cannot be shadowed.)
    """
    imp_mod = _sys.modules.get("_imp")
    locked = False
    if imp_mod is not None:
        imp_mod.acquire_lock()
        locked = True
    try:
        module = _sys.modules.get(name)
        if module is not None:
            return module
        saved_path = _list(_sys.path)
        candidates = []
        if saved_path and _isinstance(saved_path[0], _str):
            candidates.append(saved_path[0])
        if script_path and _isinstance(script_path, _str):
            cut = script_path.rfind("/")
            cut_windows = script_path.rfind("\\")
            if cut_windows > cut:
                cut = cut_windows
            candidates.append(script_path[:cut] if cut >= 0 else ".")
        trimmed = []
        for candidate in candidates:
            trimmed.append(candidate.rstrip("/\\") or candidate)
        filtered = []
        for entry in saved_path[1:]:
            if _isinstance(entry, _str):
                if entry in ("", ".", "./"):
                    continue
                stripped = entry.rstrip("/\\") or entry
                if stripped in trimmed:
                    continue
            filtered.append(entry)
        os_mod = _sys.modules.get("os")
        if os_mod is not None and trimmed:
            resolved = []
            for candidate in trimmed:
                try:
                    resolved.append(
                        os_mod.path.normcase(os_mod.path.abspath(candidate or "."))
                    )
                except _bex:
                    pass
            kept = []
            for entry in filtered:
                if _isinstance(entry, _str):
                    try:
                        if (
                            os_mod.path.normcase(os_mod.path.abspath(entry or "."))
                            in resolved
                        ):
                            continue
                    except _bex:
                        pass
                kept.append(entry)
            filtered = kept

        frozen_external = _sys.modules.get("_frozen_importlib_external")
        frozen_bootstrap = _sys.modules.get("_frozen_importlib")
        if frozen_external is not None and frozen_bootstrap is not None:
            spec = (
                frozen_bootstrap.BuiltinImporter.find_spec(name)
                or frozen_bootstrap.FrozenImporter.find_spec(name)
                or frozen_external.PathFinder.find_spec(name, filtered)
            )
            if spec is None:
                raise ImportError("cribo runtime could not resolve " + name)
            module = frozen_bootstrap.module_from_spec(spec)
            _sys.modules[name] = module
            _sys.path = filtered
            try:
                spec.loader.exec_module(module)
            except _bex:
                _sys.modules.pop(name, None)
                raise
            finally:
                _sys.path = saved_path
            return module

        # Fallback for exotic hosts without the frozen importlib modules.
        _sys.path = filtered
        try:
            return _import(name)
        finally:
            _sys.path = saved_path
    finally:
        if locked:
            imp_mod.release_lock()


class _CriboSmStream(object):
    """Byte-at-a-time reader over an iterator of byte chunks.

    Keyword-only defaults snapshot the builtins at definition time (before any
    bundled user code runs), so later shadowing of e.g. `len` cannot break the
    reader — the same idiom cribo's generated module proxies use.
    """

    __slots__ = ("_chunks", "_buf", "_pos")

    def __init__(self, chunks, *, _iter=iter):
        self._chunks = _iter(chunks)
        self._buf = b""
        self._pos = 0

    def read_byte(self, *, _len=len, _next=next, _stop=StopIteration):
        while self._pos >= _len(self._buf):
            try:
                self._buf = _next(self._chunks)
            except _stop:
                return -1
            self._pos = 0
        value = self._buf[self._pos]
        self._pos += 1
        return value


class _CriboSourceMapRuntime(object):
    """Traceback-remapping runtime.

    All collaborators (modules, the stream class, previous hooks) are bound to
    the instance at construction time, and every method snapshots the builtins
    it needs via keyword-only defaults evaluated at class-definition time — so
    the installed hooks keep working even if bundled user code later rebinds
    any module-level name this template introduced, or common builtins such as
    `open`, `len`, or `max`.
    """

    _B64 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    _CHUNK = 8192

    def __init__(
        self,
        mode,
        bundle_file,
        expected_digest,
        os_mod,
        binascii_mod,
        threading_mod,
        traceback_mod,
        hashlib_mod,
        *,
        # Class-definition-time snapshots: the module-level helper names are
        # deleted at the end of the prologue, so they must never be looked up
        # globally at construction time (test harnesses construct instances
        # long after that cleanup).
        _sys=_cribo_sys,
        _stream_cls=_CriboSmStream,
        _sm_import=_cribo_sm_import,
    ):
        self._mode = mode
        # As-given path for frame matching (co_filename uses the invocation
        # spelling), plus a startup-anchored absolute path for file I/O so a
        # later os.chdir() in bundled code cannot orphan a relative path.
        self._bundle = bundle_file
        if bundle_file == "<stdin>":
            self._bundle_anchor = bundle_file
        else:
            absolute_bundle = os_mod.path.abspath(bundle_file)
            try:
                self._bundle_anchor = os_mod.path.realpath(absolute_bundle)
            except OSError:
                self._bundle_anchor = absolute_bundle
        # SHA-256 of this build's map, baked into the executing code itself at
        # build time — immune to any on-disk replacement of the bundle, so a
        # redeployed bundle's map can never be applied to old in-memory code.
        self._expected_digest = None
        try:
            if isinstance(expected_digest, str) and len(expected_digest) == 64:
                int(expected_digest, 16)
                self._expected_digest = expected_digest.lower()
        except ValueError:
            self._expected_digest = None
        # Marks this instance so another cribo runtime chained behind it can
        # recognize it (see _notify_custom_hook).
        self._cribo_sm_runtime_marker = True
        # Startup working directory, for anchoring a relative
        # CRIBO_SOURCE_MAPS override before user code can chdir away.
        try:
            self._startup_cwd = os_mod.getcwd()
        except OSError:
            self._startup_cwd = "."
        self._os = os_mod
        self._sys = _sys
        self._binascii = binascii_mod
        self._threading = threading_mod
        self._stream_cls = _stream_cls
        self._import = _sm_import
        # Captured at construction (before any bundled user code runs) so a
        # first-party module registering sys.modules["traceback"] later cannot
        # degrade exception formatting.
        self._traceback = traceback_mod
        self._hashlib = hashlib_mod
        # Re-entrancy guard; thread-local so a hook firing on one thread never
        # disables remapping on another.
        self._local = threading_mod.local()
        self._prev_excepthook = _sys.excepthook
        self._prev_unraisablehook = _sys.unraisablehook
        self._prev_threading_hook = threading_mod.excepthook
        # Interpreter defaults, snapshotted now so user code rebinding e.g.
        # sys.__excepthook__ later cannot make the captured previous hook look
        # custom (which would double-print) or raise from the hook.
        self._default_excepthook = _sys.__excepthook__
        self._default_unraisablehook = _sys.__unraisablehook__
        self._default_threading_hook = getattr(threading_mod, "__excepthook__", None)
        try:
            self._group_type = BaseExceptionGroup
        except NameError:  # Python < 3.11
            self._group_type = None

    def install(self):
        """Install the three hooks; the previous hooks stay chained.

        Also joins the interpreter-wide runtime registry (stored on the
        unshadowable `sys` module) so stacked bundles can merge each other's
        mappings when a traceback crosses bundle boundaries.
        """
        registry = getattr(self._sys, "_cribo_sm_runtimes", None)
        if not isinstance(registry, list):
            registry = []
            self._sys._cribo_sm_runtimes = registry
        registry.append(self)
        self._sys.excepthook = self.excepthook
        self._sys.unraisablehook = self.unraisablehook
        self._threading.excepthook = self.threading_hook

    @classmethod
    def _bootstrap(
        cls, mode, bundle_file, expected_digest, *, _sm_import=_cribo_sm_import
    ):
        """Import dependencies safely, construct, and install — fail-open.

        Any failure (however exotic the host environment) leaves the program
        running without remapping instead of aborting it at startup.
        """
        try:
            script = None if bundle_file == "<stdin>" else bundle_file
            runtime = cls(
                mode,
                bundle_file,
                expected_digest,
                _sm_import("os", script),
                _sm_import("binascii", script),
                _sm_import("threading", script),
                _sm_import("traceback", script),
                _sm_import("hashlib", script),
            )
            runtime.install()
        except BaseException:
            pass

    # -- map location and raw chunk access ---------------------------------

    def _map_location(self):
        """Resolve the map location per delivery mode, or None when inactive.

        Returns (map_path, map_dir, verify); map_path is None for inline mode
        (the map lives inside the bundle file itself) and `verify` says whether
        the sibling map must match the digest recorded in the bundle. Called
        lazily at hook-fire time so the happy path never touches the
        environment or the filesystem.
        """
        env = self._os.environ.get("CRIBO_SOURCE_MAPS", "")
        if env == "0":
            return None
        # An explicit path wins for every mode (and skips digest verification —
        # it is the user's explicit choice). This is also the only way to
        # supply a map to a bundle executed via `python -` (stdin), whose
        # source cannot be re-read at hook time. A relative override is
        # anchored to the startup working directory, immune to later chdir.
        if env not in ("", "1", "true", "yes", "on"):
            path = env
            if not self._os.path.isabs(path):
                path = self._os.path.join(self._startup_cwd, path)
            return (path, self._os.path.dirname(self._os.path.abspath(path)), False)
        bundle = self._bundle_anchor
        if self._mode == "inline":
            if bundle == "<stdin>":
                return None  # stdin cannot be re-opened; use CRIBO_SOURCE_MAPS=<path>
            return (None, self._os.path.dirname(bundle), True)
        sibling = bundle + ".map"
        if self._mode == "linked":
            if self._os.path.exists(sibling):
                return (sibling, self._os.path.dirname(sibling), True)
            return None
        # external: opt in via CRIBO_SOURCE_MAPS=1 (a path was handled above)
        if env in ("1", "true", "yes", "on"):
            return (sibling, self._os.path.dirname(sibling), True)
        return None

    def _file_chunks(self, path, *, _open=open):
        """Yield fixed-size chunks of a file (constant memory)."""
        handle = _open(path, "rb")
        try:
            while True:
                chunk = handle.read(self._CHUNK)
                if not chunk:
                    break
                yield chunk
        finally:
            handle.close()

    def _handle_chunks(self, handle):
        """Yield fixed-size chunks from an already-open handle, from offset 0.

        Keeping one open handle across the verify/decode passes pins the inode:
        a concurrent build renaming a new map into place cannot swap the bytes
        between passes.
        """
        handle.seek(0)
        while True:
            chunk = handle.read(self._CHUNK)
            if not chunk:
                break
            yield chunk

    def _find_marker_tail(self, handle, marker, *, _len=len):
        """Backward-scan a file for the last occurrence of `marker`.

        Returns the offset of the payload following the marker, or -1. Only the
        tail of the file is examined; the body is never read.
        """
        handle.seek(0, 2)
        position = handle.tell()
        overlap = b""
        while position > 0:
            step = self._CHUNK if position >= self._CHUNK else position
            position -= step
            handle.seek(position)
            data = handle.read(step) + overlap
            index = data.rfind(marker)
            if index >= 0:
                return position + index + _len(marker)
            overlap = data[: _len(marker) - 1]
        return -1

    def _find_inline_payload(self, handle):
        """Offset of the inline map's base64 payload in the bundle, or -1."""
        found = self._find_marker_tail(handle, b"# sourceMappingURL=data:")
        if found < 0:
            return -1
        handle.seek(found)
        head = handle.read(192)
        base64_at = head.find(b"base64,")
        if base64_at < 0:
            return -1
        return found + base64_at + 7

    def _map_matches_digest(self, chunks_factory, *, _bex=BaseException):
        """Verify map bytes when this build carries an expected digest.

        The SHA-256 of this build's map is baked into the executing code at
        build time, so it is immune to any on-disk replacement of the bundle —
        no interleaving of concurrent builds, manual file shuffling, or
        redeploy-while-running can pair these code objects with another
        build's mappings. Hashing streams the same chunk source later used for
        decoding. When an expected digest exists, verification errors fail
        closed: bytes that were not successfully authenticated are never used.
        """
        expected = self._expected_digest
        if expected is None:
            return True
        try:
            hasher = self._hashlib.sha256()
            for chunk in chunks_factory():
                hasher.update(chunk)
            return hasher.hexdigest() == expected
        except _bex:
            return False

    def _inline_chunks(self, handle, *, _len=len):
        """Yield decoded chunks of an inline (base64 data URL) source map.

        Reads from an already-open bundle handle so all passes see one inode.
        """
        start = self._find_inline_payload(handle)
        if start < 0:
            return
        handle.seek(start)
        pending = b""
        while True:
            raw = handle.read(self._CHUNK)
            if not raw:
                break
            data = pending + raw.translate(None, b"\r\n")
            usable = _len(data) - (_len(data) % 4)
            pending = data[usable:]
            if usable:
                yield self._binascii.a2b_base64(data[:usable])
        if pending:
            yield self._binascii.a2b_base64(pending + b"=" * (-_len(pending) % 4))

    # -- streaming JSON field scanner ---------------------------------------

    def _skip_ws(self, stream, byte):
        while byte in (32, 9, 10, 13):
            byte = stream.read_byte()
        return byte

    def _read_hex4(
        self, stream, *, _int=int, _chr=chr, _range=range, _error=ValueError
    ):
        """Read the four hex digits of a \\uXXXX escape; return the code unit."""
        code = 0
        for _ in _range(4):
            digit = stream.read_byte()
            if digit < 0:
                raise _error("unterminated unicode escape")
            code = code * 16 + _int(_chr(digit), 16)
        return code

    def _read_string(
        self,
        stream,
        collect,
        *,
        _bytearray=bytearray,
        _chr=chr,
        _error=ValueError,
    ):
        """Consume a JSON string whose opening quote was already read.

        Returns the decoded text when collect is true, else None (contents are
        discarded byte-by-byte). Escaped quotes, \\uXXXX sequences, and UTF-16
        surrogate pairs (non-BMP characters) are handled, so string *values*
        containing text like '"mappings":' cannot confuse the key scanner and
        emoji-bearing paths survive intact.
        """
        table = {98: 8, 102: 12, 110: 10, 114: 13, 116: 9}
        buf = _bytearray() if collect else None
        pending = None  # one byte of lookahead pushed back by surrogate handling
        while True:
            if pending is None:
                byte = stream.read_byte()
            else:
                byte, pending = pending, None
            if byte < 0:
                raise _error("unterminated JSON string")
            if byte == 34:  # '"'
                return buf.decode("utf-8", "replace") if collect else None
            if not byte == 92:  # '\\'
                if buf is not None:
                    buf.append(byte)
                continue
            escape = stream.read_byte()
            if escape < 0:
                raise _error("unterminated JSON escape")
            if not escape == 117:  # 'u'
                if buf is not None:
                    buf.append(table.get(escape, escape))
                continue
            code = self._read_hex4(stream)
            if buf is None:
                continue
            if 0xD800 <= code <= 0xDBFF:
                # High surrogate: a following \uXXXX low surrogate combines
                # into one non-BMP code point.
                nxt = stream.read_byte()
                if nxt == 92:
                    escape2 = stream.read_byte()
                    if escape2 == 117:
                        low = self._read_hex4(stream)
                        if 0xDC00 <= low <= 0xDFFF:
                            code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00)
                            buf.extend(_chr(code).encode("utf-8"))
                        else:
                            buf.extend(_chr(code).encode("utf-8", "surrogatepass"))
                            buf.extend(_chr(low).encode("utf-8", "surrogatepass"))
                        continue
                    if escape2 < 0:
                        raise _error("unterminated JSON escape")
                    buf.extend(_chr(code).encode("utf-8", "surrogatepass"))
                    buf.append(table.get(escape2, escape2))
                    continue
                buf.extend(_chr(code).encode("utf-8", "surrogatepass"))
                pending = nxt  # includes EOF/quote; the main loop handles both
                continue
            buf.extend(_chr(code).encode("utf-8", "surrogatepass"))

    def _skip_value(self, stream, byte, *, _error=ValueError):
        """Skip one JSON value; return the first byte after it (or -1)."""
        if byte == 34:  # string
            self._read_string(stream, False)
            return stream.read_byte()
        if byte in (123, 91):  # object / array
            depth = 1
            while depth > 0:
                byte = stream.read_byte()
                if byte < 0:
                    raise _error("unterminated JSON container")
                if byte == 34:
                    self._read_string(stream, False)
                elif byte in (123, 91):
                    depth += 1
                elif byte in (125, 93):
                    depth -= 1
            return stream.read_byte()
        # number / true / false / null: consume until a delimiter
        while byte >= 0 and byte not in (44, 125, 93):  # ',' '}' ']'
            byte = stream.read_byte()
        return byte

    def _decode_vlq(
        self, stream, needed, max_needed, *, _range=range, _len=len, _error=ValueError
    ):
        """Streaming VLQ state machine over the raw bytes of the mappings string.

        Constant state: line/segment counters plus running deltas. Records the
        first segment per needed generated line; exits as soon as every needed
        line is resolved or the max needed line is passed. Returns
        ``(table, terminated)`` where ``terminated`` says whether the closing
        quote was consumed (early exits leave the stream inside the string).
        """
        lut = {}
        for index in _range(64):
            lut[self._B64[index]] = index
        result = {}
        gen_line = 0
        src_idx = 0
        src_line = 0
        name_idx = 0
        field = 0
        vlq_value = 0
        vlq_shift = 0

        def end_segment():
            if field >= 4 and gen_line in needed and gen_line not in result:
                result[gen_line] = (
                    src_idx,
                    src_line,
                    name_idx if field >= 5 else None,
                )

        while True:
            byte = stream.read_byte()
            if byte < 0 or byte == 34:  # EOF or closing '"'
                end_segment()
                return result, True
            if byte == 59:  # ';'
                end_segment()
                gen_line += 1
                field = 0
                if gen_line > max_needed or _len(result) == _len(needed):
                    return result, False
                continue
            if byte == 44:  # ','
                end_segment()
                field = 0
                continue
            value = lut.get(byte)
            if value is None:
                raise _error("unexpected byte in mappings")
            vlq_value += (value & 31) << vlq_shift
            if value & 32:
                vlq_shift += 5
                continue
            signed = -(vlq_value >> 1) if vlq_value & 1 else vlq_value >> 1
            if field == 1:
                src_idx += signed
            elif field == 2:
                src_line += signed
            elif field == 4:
                name_idx += signed
            field += 1
            vlq_value = 0
            vlq_shift = 0

    def _scan(self, chunks_factory, needed, max_needed, *, _set=set, _error=ValueError):
        """Streaming scan; return (sources dict, names dict, line table).

        Pass 1 decodes only `mappings` (skipping the sources array entirely);
        later passes collect only the source paths and symbol names referenced
        by decoded lines. Memory therefore scales with the traceback's needed
        frames, not with the bundle's full source/name tables.
        `chunks_factory` produces a fresh chunk iterator per pass.
        """
        table = self._scan_mappings(
            self._stream_cls(chunks_factory()), needed, max_needed
        )
        wanted_sources = _set(src_idx for (src_idx, _line, _name_idx) in table.values())
        wanted_names = _set(
            name_idx
            for (_src_idx, _line, name_idx) in table.values()
            if name_idx is not None
        )
        sources = {}
        names = {}
        if wanted_sources:
            sources = self._scan_string_array(
                self._stream_cls(chunks_factory()), "sources", wanted_sources
            )
        if wanted_names:
            names = self._scan_string_array(
                self._stream_cls(chunks_factory()), "names", wanted_names
            )
        return sources, names, table

    def _scan_mappings(self, stream, needed, max_needed, *, _error=ValueError):
        """Pass 1: decode the `mappings` field; every other value is skipped."""
        byte = self._skip_ws(stream, stream.read_byte())
        if not byte == 123:  # '{'
            raise _error("not a JSON object")
        byte = self._skip_ws(stream, stream.read_byte())
        while byte == 34:  # '"' starting a key
            key = self._read_string(stream, True)
            byte = self._skip_ws(stream, stream.read_byte())
            if not byte == 58:  # ':'
                raise _error("malformed object")
            byte = self._skip_ws(stream, stream.read_byte())
            if key == "mappings":
                if not byte == 34:
                    raise _error("mappings is not a string")
                table, _terminated = self._decode_vlq(stream, needed, max_needed)
                return table
            byte = self._skip_ws(stream, self._skip_value(stream, byte))
            if byte == 44:  # ','
                byte = self._skip_ws(stream, stream.read_byte())
        raise _error("no mappings field")

    def _scan_string_array(self, stream, field_name, wanted, *, _error=ValueError):
        """Collect selected string items from one top-level array field."""
        byte = self._skip_ws(stream, stream.read_byte())
        if not byte == 123:  # '{'
            raise _error("not a JSON object")
        byte = self._skip_ws(stream, stream.read_byte())
        while byte == 34:  # '"' starting a key
            key = self._read_string(stream, True)
            byte = self._skip_ws(stream, stream.read_byte())
            if not byte == 58:  # ':'
                raise _error("malformed object")
            byte = self._skip_ws(stream, stream.read_byte())
            if key == field_name:
                if not byte == 91:  # '['
                    raise _error(field_name + " is not an array")
                return self._read_wanted_array_items(stream, wanted)
            byte = self._skip_ws(stream, self._skip_value(stream, byte))
            if byte == 44:  # ','
                byte = self._skip_ws(stream, stream.read_byte())
        return {}

    def _read_wanted_array_items(self, stream, wanted, *, _len=len, _error=ValueError):
        """Read a JSON array, collecting only string items at wanted indices."""
        items = {}
        index = 0
        byte = self._skip_ws(stream, stream.read_byte())
        if byte == 93:  # ']'
            return items
        while True:
            if byte == 34 and index in wanted:
                items[index] = self._read_string(stream, True)
                byte = self._skip_ws(stream, stream.read_byte())
            else:
                byte = self._skip_ws(stream, self._skip_value(stream, byte))
            index += 1
            if byte == 93 or _len(items) == _len(wanted):
                return items
            if not byte == 44:  # ','
                raise _error("malformed array")
            byte = self._skip_ws(stream, stream.read_byte())

    # -- loading -------------------------------------------------------------

    def _load(self, needed_lines, *, _open=open, _set=set, _max=max):
        """Load (table, sources, names, map_dir) for bundle line numbers.

        Returns None when the runtime is inactive for the current mode. The
        returned table is keyed by 1-based bundle lines mapping to
        (source_index, 1-based original line, optional name_index). The map is
        opened exactly once: digest verification and all decode passes read
        from the same pinned handle, so the verified bytes are the decoded
        bytes even if a concurrent build renames a new map into place.
        """
        location = self._map_location()
        if location is None:
            return None
        map_path, map_dir, verify = location
        needed0 = _set(line - 1 for line in needed_lines)
        max_needed = _max(needed0)
        handle = _open(self._bundle_anchor if map_path is None else map_path, "rb")
        try:
            if map_path is None:

                def chunks_factory():
                    return self._inline_chunks(handle)

            else:

                def chunks_factory():
                    return self._handle_chunks(handle)

            if verify and not self._map_matches_digest(chunks_factory):
                return None
            sources, names, table0 = self._scan(chunks_factory, needed0, max_needed)
        finally:
            handle.close()
        table = {}
        for line0, (src_idx, src_line0, name_idx) in table0.items():
            table[line0 + 1] = (src_idx, src_line0 + 1, name_idx)
        return (table, sources, names, map_dir)

    def _load_json_fallback(
        self, needed_lines, *, _open=open, _set=set, _max=max, _enumerate=enumerate
    ):
        """Fallback: full json.loads parse, reusing the VLQ machine on the result."""
        location = self._map_location()
        if location is None:
            return None
        map_path, map_dir, verify = location
        json = self._import("json")

        handle = _open(self._bundle_anchor if map_path is None else map_path, "rb")
        try:
            if map_path is None:

                def fallback_chunks():
                    return self._inline_chunks(handle)

            else:

                def fallback_chunks():
                    return self._handle_chunks(handle)

            if verify and not self._map_matches_digest(fallback_chunks):
                return None
            raw = b"".join(fallback_chunks())
        finally:
            handle.close()
        data = json.loads(raw.decode("utf-8"))
        sources = {}
        for index, source in _enumerate(data.get("sources") or []):
            sources[index] = source
        names = {}
        for index, name in _enumerate(data.get("names") or []):
            names[index] = name
        mappings = data.get("mappings") or ""
        needed0 = _set(line - 1 for line in needed_lines)
        stream = self._stream_cls([mappings.encode("ascii"), b'"'])
        table0, _terminated = self._decode_vlq(stream, needed0, _max(needed0))
        table = {}
        for line0, (src_idx, src_line0, name_idx) in table0.items():
            table[line0 + 1] = (src_idx, src_line0 + 1, name_idx)
        return (table, sources, names, map_dir)

    # -- traceback collection and rendering ----------------------------------

    def _collect_needed(
        self,
        exc_value,
        traceback_obj,
        bundle_file,
        *,
        _set=set,
        _id=id,
        _getattr=getattr,
    ):
        """1-based lines of `bundle_file` referenced by the traceback chain."""
        needed = _set()

        def add(tb):
            while tb is not None:
                if tb.tb_frame.f_code.co_filename == bundle_file:
                    needed.add(tb.tb_lineno)
                tb = tb.tb_next

        add(traceback_obj)
        exc = exc_value
        seen = _set()
        while exc is not None and _id(exc) not in seen:
            seen.add(_id(exc))
            add(_getattr(exc, "__traceback__", None))
            cause = _getattr(exc, "__cause__", None)
            context = _getattr(exc, "__context__", None)
            exc = cause if cause is not None else context
        return needed

    def _chain_has_group(
        self, exc_value, *, _set=set, _id=id, _getattr=getattr, _isinstance=isinstance
    ):
        """Whether the exception chain contains a BaseExceptionGroup.

        CPython renders groups with a dedicated nested layout; rather than
        losing the nested tracebacks, the runtime defers group rendering
        entirely to the previous hook (unremapped but complete).
        """
        if self._group_type is None:
            return False
        exc = exc_value
        seen = _set()
        while exc is not None and _id(exc) not in seen:
            seen.add(_id(exc))
            if _isinstance(exc, self._group_type):
                return True
            cause = _getattr(exc, "__cause__", None)
            if cause is not None:
                exc = cause
                continue
            # A suppressed context (`raise ... from None`) is never rendered,
            # so a group hidden there must not force the unremapped fallback.
            if _getattr(exc, "__suppress_context__", False):
                return False
            exc = _getattr(exc, "__context__", None)
        return False

    def _percent_decode(
        self,
        text,
        *,
        _int=int,
        _len=len,
        _bytearray=bytearray,
        _bytes=bytes,
        _bex=BaseException,
    ):
        """Decode source URL bytes with filesystem-safe surrogate handling."""
        if "%" not in text:
            return text
        out = _bytearray()
        position = 0
        length = _len(text)
        while position < length:
            character = text[position]
            if character == "%" and position + 2 < length:
                try:
                    out.append(_int(text[position + 1 : position + 3], 16))
                    position += 3
                    continue
                except _bex:
                    pass
            out.extend(character.encode("utf-8"))
            position += 1
        return _bytes(out).decode("utf-8", "surrogateescape")

    def _source_line(self, path, lineno, *, _open=open, _os_error=OSError):
        """Read a single 1-based line from a file without caching it."""
        try:
            handle = _open(path, "rb")
        except _os_error:
            return None
        try:
            current = 0
            for raw in handle:
                current += 1
                if current == lineno:
                    return raw.decode("utf-8", "replace").strip()
                if current > lineno:
                    break
        except _os_error:
            return None
        finally:
            handle.close()
        return None

    def _effective_tb_limit(
        self, *, _getattr=getattr, _isinstance=isinstance, _int=int
    ):
        """The application's `sys.tracebacklimit`, or None when unset/invalid."""
        limit = _getattr(self._sys, "tracebacklimit", None)
        return limit if _isinstance(limit, _int) else None

    def _write_frames(
        self,
        traceback_obj,
        maps_by_file,
        write,
        limit,
        summaries,
        *,
        _bex=BaseException,
        _len=len,
    ):
        """Write remapped frame lines, collapsing repeated frames like CPython.

        `maps_by_file` holds one (table, sources, names, map_dir) tuple per known
        bundle file, so a traceback crossing several stacked cribo bundles
        remaps every frame. Standard summaries supply PEP 657 position
        indicators; mapped frames replace their location/source line while
        retaining those diagnostic lines. Consecutive identical frames
        (recursion) print at most 3 times followed by a "[Previous line repeated
        N more times]" marker. A positive `limit` keeps only the last `limit`
        frames, matching the interpreter's `sys.tracebacklimit` handling.
        """
        index = 0
        if limit is not None:
            total = 0
            probe = traceback_obj
            while probe is not None:
                total += 1
                probe = probe.tb_next
            skip = total - limit
            while skip > 0 and traceback_obj is not None:
                traceback_obj = traceback_obj.tb_next
                skip -= 1
                index += 1

        cache = {}
        last = None
        repeats = 0

        def summary_lines(frame_index):
            if summaries is None or frame_index >= _len(summaries):
                return None
            try:
                rendered = self._traceback.StackSummary.from_list(
                    [summaries[frame_index]]
                ).format()
                return "".join(rendered).splitlines(True)
            except _bex:
                return None

        def emit(entry, frame_index, mapped_frame):
            rendered_lines = summary_lines(frame_index)
            if not mapped_frame and rendered_lines:
                for line in rendered_lines:
                    write(line)
                return
            write('  File "%s", line %d, in %s\n' % entry)
            key = (entry[0], entry[1])
            if key not in cache:
                try:
                    cache[key] = self._source_line(entry[0], entry[1])
                except _bex:
                    cache[key] = None
            source_line = cache[key]
            if source_line:
                write("    %s\n" % source_line)
                # The first two standard lines are the original bundle header
                # and source text. Any remaining lines are PEP 657 carets/ranges,
                # which remain useful after substituting mapped coordinates.
                if mapped_frame and rendered_lines and _len(rendered_lines) > 2:
                    for line in rendered_lines[2:]:
                        write(line)

        while traceback_obj is not None:
            frame = traceback_obj.tb_frame
            filename = frame.f_code.co_filename
            lineno = traceback_obj.tb_lineno
            name = frame.f_code.co_name
            mapped_frame = False
            bundle_map = maps_by_file.get(filename)
            if bundle_map is not None:
                table, sources, names, map_dir = bundle_map
                mapped = table.get(lineno)
                if mapped is not None:
                    source = sources.get(mapped[0])
                    if source:
                        source = self._percent_decode(source)
                        if not self._os.path.isabs(source):
                            source = self._os.path.normpath(
                                self._os.path.join(map_dir, source)
                            )
                        filename, lineno = source, mapped[1]
                        if mapped[2] is not None:
                            original_name = names.get(mapped[2])
                            if original_name:
                                name = original_name
                        mapped_frame = True
            entry = (filename, lineno, name)
            if entry == last:
                repeats += 1
                if repeats <= 3:
                    emit(entry, index, mapped_frame)
            else:
                if repeats > 3:
                    write("  [Previous line repeated %d more times]\n" % (repeats - 3))
                last = entry
                repeats = 1
                emit(entry, index, mapped_frame)
            traceback_obj = traceback_obj.tb_next
            index += 1
        if repeats > 3:
            write("  [Previous line repeated %d more times]\n" % (repeats - 3))

    def _exception_line(
        self, exc_value, *, _type=type, _getattr=getattr, _str=str, _bex=BaseException
    ):
        """Minimal `Type: message` line, the fallback formatter."""
        exc_type = _type(exc_value)
        name = _getattr(exc_type, "__qualname__", exc_type.__name__)
        module = _getattr(exc_type, "__module__", None)
        if module not in (None, "builtins", "__main__"):
            name = "%s.%s" % (module, name)
        try:
            text = _str(exc_value)
        except _bex:
            text = "<exception str() failed>"
        return "%s: %s\n" % (name, text) if text else "%s\n" % name

    def _write_exception_only(
        self, exc, write, *, _type=type, _getattr=getattr, _bex=BaseException
    ):
        """Write the exception line(s) with full standard-library fidelity.

        `traceback.format_exception_only` supplies the interpreter's
        specialized rendering — SyntaxError source line and caret, NameError /
        AttributeError "Did you mean" suggestions, and `__notes__` — so the
        remapped output matches the default hook. Falls back to the minimal
        line (plus notes) when the traceback module is unavailable.
        """
        try:
            te = self._traceback.TracebackException(
                _type(exc),
                exc,
                _getattr(exc, "__traceback__", None),
                lookup_lines=False,
            )
            for line in te.format_exception_only():
                write(line)
            return
        except _bex:
            pass
        write(self._exception_line(exc))
        notes = _getattr(exc, "__notes__", None)
        if notes:
            try:
                for note in notes:
                    write("%s\n" % (note,))
            except _bex:
                pass

    def _render(
        self,
        exc_value,
        traceback_obj,
        maps_by_file,
        write,
        *,
        _getattr=getattr,
        _set=set,
        _id=id,
        _list=list,
        _reversed=reversed,
        _enumerate=enumerate,
        _type=type,
        _bex=BaseException,
    ):
        """Render the exception (with its cause/context chain) like CPython."""
        chain = []
        exc = exc_value
        seen = _set()
        while exc is not None and _id(exc) not in seen:
            seen.add(_id(exc))
            cause = _getattr(exc, "__cause__", None)
            context = _getattr(exc, "__context__", None)
            suppress = _getattr(exc, "__suppress_context__", False)
            if cause is not None:
                chain.append((exc, "cause"))
                exc = cause
            elif context is not None and not suppress:
                chain.append((exc, "context"))
                exc = context
            else:
                chain.append((exc, None))
                exc = None
        # Print innermost first, like CPython. The link stored on an exception
        # describes its relation to its own inner exception — which is exactly
        # the one printed immediately before it.
        ordered = _list(_reversed(chain))
        for index, (exc, link) in _enumerate(ordered):
            if index > 0:
                if link == "cause":
                    write(
                        "\nThe above exception was the direct cause of the following "
                        "exception:\n\n"
                    )
                else:
                    write(
                        "\nDuring handling of the above exception, another exception "
                        "occurred:\n\n"
                    )
            # The installed hook receives an explicit traceback that may differ
            # from (or outlive) exc_value.__traceback__. Use that argument for
            # the visible top-level exception; chained exceptions keep their
            # own traceback attributes.
            tb = (
                traceback_obj
                if exc is exc_value
                else _getattr(exc, "__traceback__", None)
            )
            limit = self._effective_tb_limit()
            if tb is not None and (limit is None or limit > 0):
                # Standard per-frame summaries (with PEP 657 positions on
                # 3.11+) render the frames this runtime does not remap. The
                # extraction passes an explicit limit spanning the whole
                # traceback: left implicit, the traceback module would honor a
                # positive sys.tracebacklimit by keeping the FIRST n frames,
                # while this renderer (like the interpreter's C printer) keeps
                # the LAST n — the truncated list would misalign or fall out
                # of range against raw traceback indices in _write_frames,
                # which applies the real limit itself.
                total_frames = 0
                probe = tb
                while probe is not None:
                    total_frames += 1
                    probe = probe.tb_next
                try:
                    summaries = self._traceback.TracebackException(
                        _type(exc), exc, tb, limit=total_frames, lookup_lines=False
                    ).stack
                except _bex:
                    summaries = None
                write("Traceback (most recent call last):\n")
                self._write_frames(tb, maps_by_file, write, limit, summaries)
            self._write_exception_only(exc, write)

    def _try_render(
        self,
        exc_value,
        traceback_obj,
        prefix,
        *,
        _getattr=getattr,
        _bex=BaseException,
        _set=set,
        _len=len,
    ):
        """Attempt a remapped rendering to stderr; True on success.

        Never raises and never masks the original exception: any failure in
        this runtime returns False so callers can delegate to the previous
        hook.
        """
        if _getattr(self._local, "in_hook", False) or exc_value is None:
            return False
        self._local.in_hook = True
        try:
            if self._chain_has_group(exc_value):
                return False
            # Every stacked cribo runtime contributes mappings for its own
            # bundle, so a traceback crossing several bundles remaps fully.
            # When two runtimes share one filename spelling (e.g. both loaded
            # as a relative "bundle.py" from different directories, or two
            # builds loaded from the same replaced path), frames carrying that
            # spelling are ambiguous. Anchor plus baked map digest identifies a
            # build; conflicting identities decline rather than apply the wrong map.
            identities_by_name = {}
            for runtime in self._registered_runtimes():
                bundle_file = _getattr(runtime, "_bundle", None)
                anchor = _getattr(runtime, "_bundle_anchor", None)
                digest = _getattr(runtime, "_expected_digest", None)
                identities_by_name.setdefault(bundle_file, _set()).add((anchor, digest))
            maps_by_file = {}
            for runtime in self._registered_runtimes():
                try:
                    bundle_file = runtime._bundle
                    if bundle_file in maps_by_file:
                        continue
                    if _len(identities_by_name.get(bundle_file, ())) > 1:
                        continue  # ambiguous spelling: skip these frames
                    needed = self._collect_needed(exc_value, traceback_obj, bundle_file)
                    if not needed:
                        continue
                    loaded = None
                    try:
                        loaded = runtime._load(needed)
                    except _bex:
                        try:
                            loaded = runtime._load_json_fallback(needed)
                        except _bex:
                            loaded = None
                    if loaded and loaded[0]:
                        maps_by_file[bundle_file] = loaded
                except _bex:
                    continue
            if not maps_by_file:
                return False
            # Buffer the rendering so a mid-render failure produces no partial
            # output before the previous hook prints the standard traceback.
            parts = []
            if prefix:
                parts.append(prefix)
            self._render(exc_value, traceback_obj, maps_by_file, parts.append)
            stderr = self._sys.stderr
            stderr.write("".join(parts))
            try:
                stderr.flush()
            except _bex:
                pass
            return True
        except _bex:
            return False
        finally:
            self._local.in_hook = False

    def _registered_runtimes(
        self, *, _getattr=getattr, _isinstance=isinstance, _list=list
    ):
        """All installed cribo runtimes in this interpreter, self included.

        The registry lives on the (unshadowable) `sys` module so stacked
        bundles can find each other's mappings.
        """
        registry = _getattr(self._sys, "_cribo_sm_runtimes", None)
        runtimes = []
        if _isinstance(registry, _list):
            for runtime in registry:
                if _getattr(runtime, "_cribo_sm_runtime_marker", False):
                    runtimes.append(runtime)
        if self not in runtimes:
            runtimes.append(self)
        return runtimes

    def _notify_custom_hook(
        self,
        prev,
        default,
        call,
        prev_attr,
        default_attr,
        *,
        _bex=BaseException,
        _getattr=getattr,
        _set=set,
        _id=id,
    ):
        """Invoke a chained hook after a successful remap when it is custom.

        A successful remap replaces the *default* printer, but preinstalled
        custom hooks (error reporters, sitecustomize) must still observe the
        exception; their own output is theirs to manage. Earlier cribo
        runtimes in the chain are traversed (via their own captured
        predecessor, named by `prev_attr`) rather than invoked, so a custom
        hook installed before any bundle still gets notified while the default
        printer is never reached (which would duplicate the traceback). Each
        traversed runtime's own captured default (named by `default_attr`) is
        collected along the way: an earlier runtime may have snapshotted a
        different default object than this one (e.g. after someone swapped
        `sys.__excepthook__` between bundle imports), and a chain ending on
        *any* of those defaults must invoke nothing — every default is a
        traceback printer, and the remap already printed. Cycle detection
        guards the traversal, and a chain that still ends on a cribo hook
        likewise invokes nothing (its registry-aware renderer would print the
        traceback a second time). When the interpreter default is unavailable
        for comparison (e.g. `threading.__excepthook__` before Python 3.10) no
        notification happens — better to skip a custom hook than to
        double-print via the default one.
        """
        defaults = [default]
        seen = _set()
        while prev is not None and _id(prev) not in seen:
            seen.add(_id(prev))
            bound_to = _getattr(prev, "__self__", None)
            if bound_to is not None and _getattr(
                bound_to, "_cribo_sm_runtime_marker", False
            ):
                other_default = _getattr(bound_to, default_attr, None)
                if other_default is not None:
                    defaults.append(other_default)
                prev = _getattr(bound_to, prev_attr, None)
                continue
            break
        bound_to = _getattr(prev, "__self__", None)
        if bound_to is not None and _getattr(
            bound_to, "_cribo_sm_runtime_marker", False
        ):
            return
        if default is None or prev is None:
            return
        for known_default in defaults:
            if prev is known_default:
                return
        try:
            call(prev)
        except _bex:
            pass

    # -- installed hooks ------------------------------------------------------

    def excepthook(self, exc_type, exc_value, traceback_obj):
        if self._try_render(exc_value, traceback_obj, None):
            self._notify_custom_hook(
                self._prev_excepthook,
                self._default_excepthook,
                lambda hook: hook(exc_type, exc_value, traceback_obj),
                "_prev_excepthook",
                "_default_excepthook",
            )
            return
        self._prev_excepthook(exc_type, exc_value, traceback_obj)

    def threading_hook(
        self, args, *, _getattr=getattr, _issubclass=issubclass, _system_exit=SystemExit
    ):
        # The default threading hook deliberately ignores SystemExit (normal
        # sys.exit() in a worker thread); preserve that by delegating.
        if args.exc_type is not None and _issubclass(args.exc_type, _system_exit):
            self._prev_threading_hook(args)
            return
        thread = _getattr(args, "thread", None)
        name = _getattr(thread, "name", None) or "Thread"
        prefix = "Exception in thread %s:\n" % name
        if self._try_render(args.exc_value, args.exc_traceback, prefix):
            self._notify_custom_hook(
                self._prev_threading_hook,
                self._default_threading_hook,
                lambda hook: hook(args),
                "_prev_threading_hook",
                "_default_threading_hook",
            )
            return
        self._prev_threading_hook(args)

    def unraisablehook(self, unraisable, *, _getattr=getattr, _bex=BaseException):
        message = _getattr(unraisable, "err_msg", None) or "Exception ignored in"
        try:
            prefix = "%s: %r\n" % (message, unraisable.object)
        except _bex:
            prefix = "%s\n" % message
        if self._try_render(unraisable.exc_value, unraisable.exc_traceback, prefix):
            self._notify_custom_hook(
                self._prev_unraisablehook,
                self._default_unraisablehook,
                lambda hook: hook(unraisable),
                "_prev_unraisablehook",
                "_default_unraisablehook",
            )
            return
        self._prev_unraisablehook(unraisable)


_CriboSourceMapRuntime._bootstrap(
    "__CRIBO_SOURCEMAP_MODE__",
    globals().get("__file__", "<stdin>"),
    "__CRIBO_SOURCEMAP_DIGEST__",
)
# Leave no trace in the bundle's namespace: user code must not see (or
# collide with) runtime helpers, and `from bundle import *` must not export
# them. The installed instance holds every reference it needs (captured in
# __init__/keyword defaults), so nothing here is looked up globally after
# bootstrap.
del _cribo_sys, _cribo_sm_import, _CriboSmStream, _CriboSourceMapRuntime
