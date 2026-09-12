# Source Map v3 Support

Status: implemented. This document records the agreed design plus the
implementation notes at the end.

## Problem Statement

Cribo produces a single bundled `.py` file; when that bundle raises an exception,
tracebacks point at bundle line numbers instead of the original source files, making
debugging painful. This feature adds opt-in Source Map v3 generation (the
language-agnostic JS-ecosystem format) plus an injected Python runtime that remaps
tracebacks to original sources at run time, analogous to `node --enable-source-maps`.

## Requirements

1. **Granularity:** statement/line-level mappings (column 0) — sufficient for Python's
   line-oriented tracebacks.
2. **Delivery:** esbuild-style `--sourcemap[=linked|inline|external]` (bare flag =
   `linked`).
   - `linked`: write `<output>.map` next to the output and append a
     `# sourceMappingURL=<basename>.map` comment as the last line.
     A newly created map file is owner-only (`0600` on Unix, set atomically at
     creation) since it may embed `sourcesContent`; rebuilds preserve whatever
     permissions the existing map carries, so a one-time `chmod` sticks.
   - `inline`: append a `# sourceMappingURL=data:application/json;base64,...` comment.
   - `external`: write the `.map` file with no comment.
3. **Runtime traceback injection** is bundled into the output whenever `--sourcemap`
   is used; activation depends on the mode:
   - `inline` → active by default (`CRIBO_SOURCE_MAPS=0` acts as a kill-switch).
   - `external` → gated on the env var `CRIBO_SOURCE_MAPS=1` (a value that is a path
     is treated as the map file location).
   - `linked` → active iff the sibling `.map` file exists at run time; silently
     disabled otherwise.
4. **Crate:** `oxc_sourcemap` v8.x (pinned exact version), BSD-3-Clause, maintained at
   `oxc-project/oxc-sourcemap`, used by rolldown/rspack. Its dependencies
   (`rustc-hash 2`, `serde`, `serde_json`, `base64-simd`, `json-escape-simd`) align
   with the workspace; MSRV 1.95 is below the repo toolchain 1.97.1.
5. **`sourcesContent`:** the default depends on the mode — omitted for `inline`,
   included for `linked`/`external`; forcible override via `--sources-content=true|false`
   (and the matching config key).
6. **Hook scope:** full set — `sys.excepthook`, `threading.excepthook`,
   `sys.unraisablehook`; chain to previously installed hooks; never crash (fail open
   to default behavior).
7. **Laziness:** zero-to-negligible happy-path cost. The prologue does NO I/O, no
   parsing, no decoding at startup — it only defines functions, stores `__file__` and
   the bundle-time mode constant, and installs the 3 hooks. Activation gating itself
   (map existence check, env var read, data-URL location) is deferred to the first
   exception.
8. **Duress tolerance:** the decoder must work under resource-constrained conditions
   (`MemoryError`, `RecursionError`, FD exhaustion) — streaming, constant-memory
   implementation; performance is secondary on the error path.

## Design

### Mapping extraction (Rust side)

Cribo emits the bundle by unparsing the merged AST statement-by-statement with ruff's
`Generator` (`orchestrator.rs::bundle_to_string` →
`code_generator/python_codegen.rs::generate_statement`). Ruff codegen emits no
position info, so mappings cannot be captured during emission.

Instead, cribo re-parses the final bundle text once with ruff's parser and performs a
defensive parallel statement-only walk against the bundled AST (which carries node
provenance via node indices and original `TextRange`s). Each aligned statement with
provenance yields one mapping: generated line → (original file, original line). On
structural divergence (e.g., the class-pattern rewriter's string patching), the walk
logs at debug level and skips the subtree rather than failing. The re-parse doubles
as a bundle-validity check.

Provenance groundwork already exists:

- `ast_indexer.rs` gives each module a 1,000,000-wide node-index range
  (`node_index / MODULE_INDEX_RANGE` = module ordinal); statements copied from module
  ASTs retain their original `TextRange`.
- Synthesized nodes get indices from `transformation_context.rs` and are
  distinguishable (no original range) — they produce no mapping.

Key components:

- `crates/cribo/src/source_map.rs`: mapping record builder wrapping
  `oxc_sourcemap::SourceMapBuilder`, provenance resolution, and the parallel walk.
- Each parsed module's source text plus a line-offset index is retained (keyed by
  `ModuleId`) through `bundle_core` — needed both for offset→line conversion and for
  `sourcesContent`.
- `sources` paths are recorded relative to the map location (esbuild convention),
  with an empty `sourceRoot`.
- `--stdout` interplay: bare `--sourcemap` with `--stdout` defaults to `inline`;
  explicit `linked`/`external` with `--stdout` is an error suggesting `inline`.
- Config-file (`cribo.toml`) equivalents: `sourcemap = "linked" | "inline" | "external"`,
  `sources-content = true | false`.

### Runtime prologue (Python side) — lazy, streaming, duress-tolerant

The template lives in `crates/cribo/src/python/` and is embedded via `include_str!`,
injected as a prologue when source maps are enabled, with the delivery mode baked in
as a constant.

**Happy path:** define functions + 3 hook assignments (capturing previous hooks for
chaining). Nothing else — no file I/O, no parsing, no decoding.

**Exception path pipeline:**

1. **Collect needed lines.** Walk the `tb_next` chain (plus `__cause__`/`__context__`)
   collecting `tb_lineno` for frames whose `co_filename` matches the bundle path.
   Result: a small set of ints and `max_needed` for early exit.
2. **Locate the mappings bytes without loading the map.**
   - `linked`/`external`: stream the `.map` file in fixed 8 KiB chunks.
   - `inline`: open the bundle file, `seek()` to EOF, scan backward in chunks for the
     last newline to find the `# sourceMappingURL=data:...;base64,` line (the bundle
     body is never read), then decode the base64 payload incrementally with
     `binascii.a2b_base64` on 4-byte-aligned chunk boundaries.
3. **Targeted streaming JSON field extraction.** The map is NOT parsed with
   `json.loads`. A minimal JSON string-lexer scans the chunk stream for the top-level
   `"sources"` key (a small array, parsed into a list) and the `"mappings"` key
   (consumed by the VLQ state machine directly from the stream, never held whole in
   memory). The lexer handles escaped quotes so `sourcesContent` values containing
   fake `"mappings":` keys cannot fool it.
4. **Streaming VLQ state machine, constant memory.** State is six integers:
   `gen_line`, running deltas `src_idx` and `src_line`, VLQ accumulators `vlq_value`
   and `vlq_shift`, and a `field` counter. `;` increments `gen_line`, `,` ends a
   segment; `(gen_line → (src_idx, src_line))` is recorded only for lines in the
   needed set (first segment per line); the machine early-exits once
   `gen_line > max_needed`. Total heap: chunk buffer + six ints + k result entries.
5. **Re-render, best-effort in layers.** The file:line remap is always attempted;
   the original source-line *text* is decoration in a separate `try` — stream-read
   just the needed line from the original file on disk (iterate, never slurp; NO
   `linecache` — it caches whole files), or a second targeted pass over
   `sourcesContent`; skipped silently on failure.

**Failure containment:**

- Fallback ladder, never mask the real error: (1) streaming targeted path → (2) one
  attempt at plain `json.loads` → (3) delegate to the captured previous hook with the
  original exception. The entire hook body is wrapped catching `BaseException` raised
  by our own code.
- No imports inside the hook: `sys`, `os`, etc. are bound at prologue time. The
  renderer formats frames directly from `tb_frame.f_code` attributes mirroring
  CPython's format (self-contained; no dependency on `traceback.TracebackException`).
- Iterative code only (no recursion); the recursion limit is bumped by a small margin
  (`sys.setrecursionlimit(cur + 64)`) inside a `try` before rendering and restored
  after.
- Re-entrancy guard: a module-level flag prevents the hook recursing into itself.

**Documented caveats:**

- Under hard OOM where the interpreter cannot allocate at all, no pure-Python hook
  can run; the target is constrained-but-alive conditions with guaranteed
  non-interference at the floor.
- User code calling `traceback.format_exc()` (or otherwise formatting tracebacks
  itself) is not remapped — only the installed hooks re-render.

## Task Breakdown

- **Task 0:** this document, cross-referenced from `docs/static-bundling.md`.
- **Task 1:** `oxc_sourcemap` workspace dependency (pinned) + `source_map.rs` with a
  `SourceMapGenerator` accepting `(generated_line, source_file, original_line)`
  records and optional per-source content, serializing valid Source Map v3 JSON.
  Unit tests: mapping order, VLQ round-trip via the crate's consumer API,
  `sourcesContent` on/off, empty map.
- **Task 2:** retain module source text + line-offset index keyed by `ModuleId`;
  provenance resolver `node_index` → module ordinal → original file path and
  `TextRange.start()` → original line; `None` for synthesized nodes. Unit tests with
  multi-module inputs including a synthesized-node case.
- **Task 3:** mapping extraction via re-parse + parallel statement walk (including
  statements nested in function/class bodies and wrapper-module init functions),
  integrating Tasks 1+2 into a complete `SourceMap` per bundle. Tests assert selected
  known mappings for inlined-module, wrapper-module, and class-pattern bundles.
- **Task 4:** CLI `--sourcemap[=linked|inline|external]` (clap `ValueEnum`, bare =
  `linked`) + `cribo.toml` key; the three delivery modes including the
  `# sourceMappingURL=` trailer; `--stdout` interplay. Integration tests per mode.
- **Task 5:** `sourcesContent` mode-dependent default with `--sources-content`
  override (+ config key). Tests cover the default matrix and both overrides.
- **Task 6:** Python runtime prologue per the design above, with a pytest suite for
  the unit-testable pieces (VLQ state machine against maps generated by
  `oxc_sourcemap`, backward EOF scan, base64 chunk alignment, JSON scanner against
  adversarial `sourcesContent`) and an integration test asserting a remapped
  traceback on stderr.
- **Task 7:** `threading.excepthook` + `sys.unraisablehook`; duress tests
  (`RecursionError` at depth, `MemoryError`, FD exhaustion via
  `resource.setrlimit(RLIMIT_NOFILE, ...)`), activation-matrix tests, and a laziness
  test asserting a non-throwing run performs no map access.
- **Task 8:** snapshot-framework integration (fixtures opt into source maps,
  snapshotting remapped-traceback output), at least two fixtures, README/CLI docs,
  and caveat documentation.


## Implementation Notes

Decisions made (or refined) during implementation:

- **Prologue injection is AST-level, not text-level.** The runtime template
  (`crates/cribo/src/python/sourcemap_runtime.py`, embedded via `include_str!`)
  is parsed with ruff and its statements are spliced into the bundled AST after
  any leading `from __future__` imports, *before* code generation. The extraction
  walk therefore stays structurally aligned automatically (prologue statements
  carry no provenance and simply produce no mappings), with no line-offset
  bookkeeping.
- **Source line text comes from disk only.** The runtime resolves relative
  source paths against the map's directory and stream-reads the single needed
  line. A second streaming pass over `sourcesContent` for line text was
  deliberately skipped to bound runtime complexity — `sourcesContent` is still
  embedded per the configured policy for external tooling (IDEs, error
  trackers). When original files are absent, tracebacks still remap `file:line`
  and simply omit the source-line text.
- **Repeated frames are collapsed like CPython** (at most 3 identical
  consecutive frames, then `[Previous line repeated N more times]`), with a
  per-render line-text cache; a `RecursionError` traceback stays small and does
  not trigger thousands of file reads.
- **`threading` and `traceback` are imported at bundle startup** (through a
  shadow-proof importer that skips the script directory) so
  `threading.excepthook` can be installed and exception formatting cannot be
  degraded by a first-party module registering `sys.modules["traceback"]`;
  these are the only non-trivial startup costs and are negligible in practice.
  Map location checks, environment reads, file access, and decoding all remain
  deferred to the first exception.
- **Snapshot integration:** fixtures under `crates/cribo/tests/fixtures/` whose
  name starts with `sourcemap_` are bundled with `--sourcemap=linked` and gain a
  `source_map@<fixture>.snap` snapshot: a normalized, path-free dump of every
  mapping (`bundle:<line> `<statement>` -> <source basename>:<line>`). Remapped
  *traceback output* is asserted exactly (not snapshotted) in
  `crates/cribo/tests/test_source_maps.rs`, which also covers the activation
  matrix, thread/unraisable hooks, and the duress suite (RecursionError,
  MemoryError under `RLIMIT_AS`, FD exhaustion under `RLIMIT_NOFILE`, and
  happy-path laziness with an unreadable map).

Known limitations (documented in the README as well):

- User code that formats tracebacks itself (`traceback.format_exc()`,
  `traceback.print_exc()`, custom formatters) is not remapped; only the
  installed hooks re-render.
- Under a hard out-of-memory condition where the interpreter cannot allocate at
  all, no pure-Python hook can run; the guarantee is non-interference (the
  default traceback still prints) rather than remapping.
- Mappings are statement/line-level with column 0 by design; Python tracebacks
  are line-oriented, so finer columns would add cost without changing the
  rendered output.


## Review-Driven Refinements (PR #570)

- **The runtime is a single class** (`_CriboSourceMapRuntime`); all collaborators
  (modules, previous hooks, state) live on the instance and the installed hooks
  are bound methods, so bundled user code rebinding any injected module-level
  name cannot break remapping.
- **Custom pre-installed hooks stay notified.** After a successful remap, a
  previous `sys.excepthook`/`threading.excepthook`/`sys.unraisablehook` that
  differs from the interpreter default is still invoked (error reporters and
  `sitecustomize` integrations keep working); the default printer is the only
  thing the remap replaces.
- **`SystemExit` in worker threads stays silent**, matching the default
  `threading.excepthook`.
- **`ExceptionGroup` chains defer to the previous hook** (complete, unremapped
  rendering) rather than losing nested tracebacks.
- **The re-entrancy guard is thread-local.**
- **`CRIBO_SOURCE_MAPS=<path>` works in every mode** and is the supported way to
  remap a bundle executed via `python -` (stdin), whose own file cannot be
  re-read; without it, inline mode deactivates gracefully for `<stdin>` bundles.
- **Environment configuration:** `CRIBO_SOURCEMAP=linked|inline|external` and
  `CRIBO_SOURCES_CONTENT=<bool>` participate in the standard `CRIBO_*` layering.
- **The template's docstring is stripped at injection** so enabling source maps
  does not change the bundle's `__doc__`.
- **`elif` headers get their own mappings** (Python reports the header line when
  a condition raises), using the condition expression's provenance.
- **Linked/external maps are written only after the bundle write succeeds**, so
  a failed run cannot leave a stale map next to an older bundle.
- `relative_path` refuses lexically non-invertible bases (`..` components) and
  falls back to the absolute source path.
