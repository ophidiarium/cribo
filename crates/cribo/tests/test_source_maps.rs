//! Integration tests for `--sourcemap` CLI delivery modes.
//!
//! Each test drives the cribo binary end-to-end and inspects the emitted
//! bundle and/or `.map` file. See `docs/source-maps.md` for the design.

mod common;

use std::{fs, path::Path, process::Command};

use tempfile::TempDir;

/// Marker prefix of an inline source map comment.
const INLINE_MARKER: &str = "# sourceMappingURL=data:application/json;base64,";

/// Create a project from (file name, content) pairs and return its directory.
fn make_project(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("create temp dir");
    for (name, content) in files {
        fs::write(dir.path().join(name), content).expect("write fixture file");
    }
    dir
}

/// Create a two-module fixture project and return its directory.
fn fixture_project() -> TempDir {
    make_project(&[
        (
            "main.py",
            "from helper import greet\n\nprint(greet(\"world\"))\n",
        ),
        (
            "helper.py",
            "def greet(name):\n    message = f\"hello {name}\"\n    return message\n",
        ),
    ])
}

/// Run the cribo binary with `args`, returning (status success, stdout, stderr).
fn run_cribo(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_cribo"))
        .args(args)
        .output()
        .expect("run cribo binary");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Parse a source map JSON string and assert it is valid Source Map v3.
fn parse_map(json: &str) -> oxc_sourcemap::SourceMap<'_> {
    oxc_sourcemap::SourceMap::from_json_string(json).expect("valid Source Map v3 JSON")
}

/// Return the fixture entry path (`main.py`) as a CLI argument string.
fn entry_arg(dir: &TempDir) -> String {
    dir.path().join("main.py").to_string_lossy().into_owned()
}

/// Assert the map targets `bundle_file`, lists `helper.py`, and has mappings.
fn assert_map_covers_helper(map_json: &str, bundle_file: &str) {
    let map = parse_map(map_json);
    assert_eq!(map.get_file(), Some(bundle_file));
    assert!(
        map.get_sources()
            .any(|source| source.ends_with("helper.py")),
        "map sources must include helper.py: {:?}",
        map.get_sources().collect::<Vec<_>>()
    );
    assert!(map.get_tokens().count() > 0, "map must contain mappings");
}

#[test]
fn linked_mode_writes_map_and_comment() {
    let dir = fixture_project();
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");

    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(
        bundle
            .trim_end()
            .ends_with("# sourceMappingURL=bundle.py.map"),
        "linked mode must append the sourceMappingURL comment"
    );

    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map file");
    assert_map_covers_helper(&map_json, "bundle.py");
    // Linked mode embeds sourcesContent by default.
    assert!(map_json.contains("sourcesContent"));
}

#[test]
fn bare_sourcemap_flag_defaults_to_linked() {
    let dir = fixture_project();
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    assert!(dir.path().join("bundle.py.map").exists());
    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(bundle.contains("# sourceMappingURL=bundle.py.map"));
}

#[test]
fn external_mode_writes_map_without_comment() {
    let dir = fixture_project();
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=external",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");

    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(
        !bundle
            .lines()
            .any(|line| line.starts_with("# sourceMappingURL=")),
        "external mode must not reference the map from the bundle"
    );
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map file");
    assert_map_covers_helper(&map_json, "bundle.py");
}

#[test]
fn inline_mode_embeds_map_and_writes_no_file() {
    let dir = fixture_project();
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=inline",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    assert!(
        !dir.path().join("bundle.py.map").exists(),
        "inline mode must not write a map file"
    );

    let bundle = fs::read_to_string(&out).expect("read bundle");
    let map_json = decode_inline_map(&bundle);
    assert_map_covers_helper(&map_json, "bundle.py");
    // Inline mode omits sourcesContent by default.
    assert!(!map_json.contains("sourcesContent"));
}

#[test]
fn stdout_with_bare_sourcemap_selects_inline() {
    let dir = fixture_project();
    let (ok, stdout, stderr) = run_cribo(&["--entry", &entry_arg(&dir), "--stdout", "--sourcemap"]);
    assert!(ok, "bundling must succeed: {stderr}");
    let map_json = decode_inline_map(&stdout);
    assert_map_covers_helper(&map_json, "<stdout>");
    // A stdout bundle can be redirected anywhere, so its map has no anchor
    // directory: source paths must stay absolute.
    let map = parse_map(&map_json);
    assert!(
        map.get_sources()
            .all(|source| Path::new(source).is_absolute()),
        "stdout maps must carry absolute source paths: {:?}",
        map.get_sources().collect::<Vec<_>>()
    );
}

#[test]
fn stdout_with_linked_sourcemap_errors() {
    let dir = fixture_project();
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--stdout",
        "--sourcemap=linked",
    ]);
    assert!(!ok, "linked + stdout must be rejected");
    assert!(
        stderr.contains("--sourcemap=inline"),
        "error must suggest inline mode: {stderr}"
    );
}

#[test]
fn stdout_with_external_sourcemap_errors() {
    let dir = fixture_project();
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--stdout",
        "--sourcemap=external",
    ]);
    assert!(!ok, "external + stdout must be rejected");
    assert!(stderr.contains("--sourcemap=inline"));
}

#[test]
fn no_sourcemap_by_default() {
    let dir = fixture_project();
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    assert!(!dir.path().join("bundle.py.map").exists());
    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(!bundle.contains("sourceMappingURL"));
}

#[test]
fn config_file_sourcemap_key_is_honored() {
    let dir = fixture_project();
    fs::write(dir.path().join("cribo.toml"), "sourcemap = \"external\"\n").expect("write config");
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--config",
        &dir.path().join("cribo.toml").to_string_lossy(),
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    assert!(
        dir.path().join("bundle.py.map").exists(),
        "config-file sourcemap key must enable map emission"
    );
    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(
        !bundle
            .lines()
            .any(|line| line.starts_with("# sourceMappingURL=")),
        "external mode: no comment"
    );
}

/// Extract and decode the inline source map data URL from bundle text.
fn decode_inline_map(bundle: &str) -> String {
    let marker_pos = bundle
        .rfind(INLINE_MARKER)
        .expect("inline source map comment present");
    let payload = bundle[marker_pos + INLINE_MARKER.len()..].trim_end();
    let bytes = base64_simd::STANDARD
        .decode_to_vec(payload.as_bytes())
        .expect("valid base64 payload");
    String::from_utf8(bytes).expect("valid UTF-8 source map")
}

/// The map file sits next to the bundle even when the output path is nested.
#[test]
fn linked_map_lands_next_to_nested_output() {
    let dir = fixture_project();
    let nested = dir.path().join("dist").join("app.py");
    fs::create_dir_all(nested.parent().expect("parent")).expect("mkdir dist");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &nested.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let map_path = dir.path().join("dist").join("app.py.map");
    assert!(map_path.exists(), "map must sit next to the nested output");
    let bundle = fs::read_to_string(&nested).expect("read bundle");
    assert!(bundle.contains("# sourceMappingURL=app.py.map"));

    // Source paths must be relative to the map's directory.
    let map_json = fs::read_to_string(&map_path).expect("read map");
    let map = parse_map(&map_json);
    assert!(
        map.get_sources()
            .all(|source| Path::new(source).is_relative()),
        "sources must be relative paths: {:?}",
        map.get_sources().collect::<Vec<_>>()
    );
}

#[test]
fn sources_content_can_be_forced_on_for_inline() {
    let dir = fixture_project();
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=inline",
        "--sources-content=true",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let bundle = fs::read_to_string(&out).expect("read bundle");
    let map_json = decode_inline_map(&bundle);
    assert!(
        map_json.contains("sourcesContent"),
        "--sources-content=true must force embedding for inline maps"
    );
    let map = parse_map(&map_json);
    assert!(
        map.get_source_contents()
            .flatten()
            .any(|content| content.contains("def greet")),
        "embedded content must carry the original helper source"
    );
}

#[test]
fn sources_content_can_be_forced_off_for_linked() {
    let dir = fixture_project();
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
        "--sources-content=false",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map");
    assert!(
        !map_json.contains("sourcesContent"),
        "--sources-content=false must strip embedding for linked maps"
    );
}

#[test]
fn config_file_sources_content_key_is_honored() {
    let dir = fixture_project();
    fs::write(
        dir.path().join("cribo.toml"),
        "sourcemap = \"external\"\nsources-content = false\n",
    )
    .expect("write config");
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--config",
        &dir.path().join("cribo.toml").to_string_lossy(),
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map");
    assert!(!map_json.contains("sourcesContent"));
}

// ---------------------------------------------------------------------------
// Runtime traceback remapping (injected prologue)
// ---------------------------------------------------------------------------

/// Create a fixture whose entry crashes two calls deep inside helper.py.
fn crash_project() -> TempDir {
    make_project(&[
        ("main.py", "from helper import boom\n\nboom()\n"),
        (
            "helper.py",
            "def boom():\n    inner()\n\ndef inner():\n    raise ValueError(\"kaboom\")\n",
        ),
    ])
}

/// Bundle the crash project with the given sourcemap argument; return bundle path.
fn bundle_crash_project(dir: &TempDir, sourcemap_arg: &str) -> std::path::PathBuf {
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(dir),
        "--output",
        &out.to_string_lossy(),
        sourcemap_arg,
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    out
}

/// Run a Python file; returns (status success, stdout, stderr).
fn run_python(bundle: &Path, envs: &[(&str, &str)]) -> (bool, String, String) {
    let mut command = Command::new(common::get_python_executable());
    command.arg(bundle);
    // The env-gating tests require a clean slate; an explicit entry below wins.
    command.env_remove("CRIBO_SOURCE_MAPS");
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("run python");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Whether the selected test interpreter is at least Python 3.`minor`.
fn python_at_least(minor: u8) -> bool {
    Command::new(common::get_python_executable())
        .args([
            "-c",
            &format!("import sys; raise SystemExit(0 if sys.version_info >= (3, {minor}) else 1)"),
        ])
        .status()
        .is_ok_and(|status| status.success())
}

/// Assert stderr shows the remapped traceback pointing at original files.
fn assert_remapped(stderr: &str) {
    assert!(
        stderr.contains("helper.py\", line 5, in inner"),
        "traceback must point at helper.py:5: {stderr}"
    );
    assert!(
        stderr.contains("raise ValueError(\"kaboom\")"),
        "traceback must show the original source line: {stderr}"
    );
    assert!(
        stderr.contains("main.py\", line 3, in <module>"),
        "traceback must point at main.py:3: {stderr}"
    );
    assert!(
        !stderr.contains("bundle.py\", line"),
        "no frame should remain on bundle coordinates: {stderr}"
    );
}

/// Assert stderr is one single standard (non-remapped) traceback with no
/// runtime noise.
fn assert_standard_traceback(stderr: &str) {
    assert_eq!(
        stderr.matches("Traceback (most recent call last):").count(),
        1,
        "exactly one traceback expected: {stderr}"
    );
    assert!(
        stderr.contains("bundle.py\", line"),
        "standard traceback must show bundle coordinates: {stderr}"
    );
    assert!(
        !stderr.contains("helper.py\", line 5"),
        "no remapping must happen: {stderr}"
    );
}

#[test]
fn runtime_remaps_linked_crash() {
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok, "the crashing bundle must exit non-zero");
    assert_remapped(&stderr);
}

#[test]
fn runtime_disabled_when_linked_map_missing() {
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    fs::remove_file(dir.path().join("bundle.py.map")).expect("delete map");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert_standard_traceback(&stderr);
}

#[test]
fn runtime_remaps_inline_crash() {
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=inline");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert_remapped(&stderr);
}

#[test]
fn runtime_kill_switch_disables_remapping() {
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=inline");
    let (ok, _, stderr) = run_python(&bundle, &[("CRIBO_SOURCE_MAPS", "0")]);
    assert!(!ok);
    assert_standard_traceback(&stderr);
}

#[test]
fn runtime_external_mode_is_env_gated() {
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=external");

    // Without the env var the runtime stays dormant.
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert_standard_traceback(&stderr);

    // CRIBO_SOURCE_MAPS=1 activates it against the sibling map.
    let (ok, _, stderr) = run_python(&bundle, &[("CRIBO_SOURCE_MAPS", "1")]);
    assert!(!ok);
    assert_remapped(&stderr);

    // The env var can also point directly at a relocated map file.
    let moved = dir.path().join("elsewhere.map");
    fs::rename(dir.path().join("bundle.py.map"), &moved).expect("move map");
    let (ok, _, stderr) = run_python(
        &bundle,
        &[("CRIBO_SOURCE_MAPS", moved.to_string_lossy().as_ref())],
    );
    assert!(!ok);
    assert_remapped(&stderr);

    // With the map moved away, =1 finds nothing and stays silent.
    let (ok, _, stderr) = run_python(&bundle, &[("CRIBO_SOURCE_MAPS", "1")]);
    assert!(!ok);
    assert_standard_traceback(&stderr);
}

/// Drive the pure-Python unit tests for the runtime internals (VLQ machine,
/// JSON scanner, backward EOF scan, base64 chunk alignment, json fallback).
#[test]
fn python_runtime_unit_tests() {
    use cow_utils::CowUtils as _;

    const TEMPLATE: &str = include_str!("../src/python/sourcemap_runtime.py");
    let dir = TempDir::new().expect("create temp dir");
    let runtime_path = dir.path().join("runtime.py");
    fs::write(
        &runtime_path,
        TEMPLATE
            .cow_replace("__CRIBO_SOURCEMAP_MODE__", "external")
            .as_ref(),
    )
    .expect("write substituted runtime");

    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("python")
        .join("test_sourcemap_runtime.py");
    let output = Command::new(common::get_python_executable())
        .arg(&script)
        .arg(&runtime_path)
        .output()
        .expect("run python unit tests");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "python runtime unit tests failed:\n{stdout}\n{stderr}"
    );
    // The harness discovers its tests and reports the count itself; assert the
    // sentinel plus a sanity floor instead of duplicating the exact count here.
    assert!(stdout.contains("RUNTIME TESTS PASSED"), "{stdout}");
    assert!(
        stdout.matches("PASS test_").count() >= 10,
        "expected a healthy number of runtime unit tests: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Full hook coverage, duress conditions, and laziness
// ---------------------------------------------------------------------------

#[test]
fn runtime_remaps_thread_crash() {
    let dir = make_project(&[
        (
            "main.py",
            "import threading\nfrom helper import boom\n\nworker = \
             threading.Thread(target=boom)\nworker.start()\nworker.join()\nprint(\"done\")\n",
        ),
        (
            "helper.py",
            "def boom():\n    inner()\n\ndef inner():\n    raise ValueError(\"thread kaboom\")\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    // An uncaught exception in a non-main thread does not fail the process.
    let (ok, stdout, stderr) = run_python(&bundle, &[]);
    assert!(ok, "main thread must finish normally: {stderr}");
    assert!(stdout.contains("done"));
    assert!(
        stderr.contains("Exception in thread"),
        "threading hook must announce the thread: {stderr}"
    );
    assert!(
        stderr.contains("helper.py\", line 5, in inner"),
        "thread traceback must be remapped: {stderr}"
    );
    assert!(stderr.contains("thread kaboom"));
}

#[test]
fn runtime_remaps_unraisable_error() {
    let dir = make_project(&[
        (
            "main.py",
            "from helper import make\n\nobj = make()\ndel obj\nprint(\"done\")\n",
        ),
        (
            "helper.py",
            "class Cursed:\n    def __del__(self):\n        raise RuntimeError(\"del \
             failed\")\n\ndef make():\n    return Cursed()\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, stdout, stderr) = run_python(&bundle, &[]);
    assert!(ok, "unraisable errors must not fail the process: {stderr}");
    assert!(stdout.contains("done"));
    assert!(
        stderr.contains("Exception ignored in"),
        "unraisable hook must keep the standard preamble: {stderr}"
    );
    assert!(
        stderr.contains("helper.py\", line 3, in __del__"),
        "unraisable traceback must be remapped: {stderr}"
    );
}

#[test]
fn runtime_survives_recursion_error() {
    let dir = make_project(&[
        ("main.py", "from helper import spiral\n\nspiral()\n"),
        ("helper.py", "def spiral():\n    spiral()\n"),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(stderr.contains("RecursionError"), "{stderr}");
    assert!(
        stderr.contains("helper.py\", line 2, in spiral"),
        "recursion frames must be remapped: {stderr}"
    );
    assert!(
        stderr.contains("[Previous line repeated"),
        "repeated recursion frames must be collapsed: {stderr}"
    );
    // Collapsing keeps the output small even for ~1000 recorded frames.
    assert!(
        stderr.lines().count() < 60,
        "collapsed traceback expected, got {} lines",
        stderr.lines().count()
    );
}

#[cfg(unix)]
#[test]
fn runtime_survives_memory_pressure() {
    let dir = make_project(&[
        (
            "main.py",
            "import resource\nfrom helper import hoard\n\n_soft, hard = \
             resource.getrlimit(resource.RLIMIT_AS)\ntarget = 512 * 1024 * 1024\nif hard != \
             resource.RLIM_INFINITY:\n    target = min(target, hard)\n\
             resource.setrlimit(resource.RLIMIT_AS, (target, hard))\nhoard()\n",
        ),
        (
            "helper.py",
            "def hoard():\n    blocks = []\n    try:\n        while True:\n            \
             blocks.append(bytearray(16 * 1024 * 1024))\n    except MemoryError:\n        \
             blocks.clear()\n        raise MemoryError(\"exhausted\") from None\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(stderr.contains("MemoryError"), "{stderr}");
    // The runtime must never produce a secondary error, whichever path it took.
    assert!(
        !stderr.contains("Error in sys.excepthook"),
        "the hook must not raise: {stderr}"
    );
    assert!(
        stderr.contains("helper.py\", line 8, in hoard"),
        "MemoryError under an address-space limit should still remap: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn runtime_falls_back_cleanly_on_fd_exhaustion() {
    let dir = make_project(&[
        (
            "main.py",
            "from helper import consume_fds_and_boom\n\nconsume_fds_and_boom()\n",
        ),
        (
            "helper.py",
            "import resource\n\ndef consume_fds_and_boom():\n    _soft, hard = \
             resource.getrlimit(resource.RLIMIT_NOFILE)\n    \
             resource.setrlimit(resource.RLIMIT_NOFILE, (16, hard))\n    holders = []\n    \
             try:\n        while True:\n            holders.append(open(\"/dev/null\", \
             \"rb\"))\n    except OSError:\n        pass\n    raise ValueError(\"fd exhausted \
             kaboom\")\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    // The map cannot be opened, so the runtime must fall back to the default
    // traceback without any secondary noise.
    assert!(stderr.contains("fd exhausted kaboom"), "{stderr}");
    assert_eq!(
        stderr.matches("Traceback (most recent call last):").count(),
        1,
        "exactly one traceback expected: {stderr}"
    );
    assert!(
        !stderr.contains("Error in sys.excepthook"),
        "the hook must not raise: {stderr}"
    );
    assert!(
        !stderr.contains("helper.py\", line"),
        "with the map unreadable, frames stay on bundle coordinates: {stderr}"
    );
}

#[test]
fn runtime_tolerates_broken_map_on_happy_path() {
    let dir = fixture_project(); // non-throwing project
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");

    // Replace the map with garbage (and drop read permission on Unix, though
    // that is a no-op when running as root). The runtime is fail-open, so this
    // cannot *prove* the map is never touched — laziness itself is enforced by
    // the runtime design (all map access lives behind the hook path). What it
    // proves is that a broken or unreadable map never disturbs a successful run.
    let map_path = dir.path().join("bundle.py.map");
    fs::write(&map_path, "NOT JSON {{{").expect("overwrite map");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&map_path, fs::Permissions::from_mode(0o000))
            .expect("make map unreadable");
    }

    let (ok, stdout, stderr) = run_python(&out, &[]);
    assert!(ok, "happy-path run must succeed: {stderr}");
    assert!(stdout.contains("hello world"));
    assert!(
        stderr.is_empty(),
        "a broken map must not disturb a successful run: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Precedence, environment configuration, and hook-chaining behavior
// ---------------------------------------------------------------------------

#[test]
fn cli_sourcemap_flag_overrides_config_file() {
    let dir = fixture_project();
    fs::write(dir.path().join("cribo.toml"), "sourcemap = \"external\"\n").expect("write config");
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--config",
        &dir.path().join("cribo.toml").to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(
        bundle.contains("# sourceMappingURL=bundle.py.map"),
        "CLI --sourcemap=linked must override the config file's external mode"
    );
}

#[test]
fn env_var_enables_sourcemap_generation() {
    let dir = fixture_project();
    let out = dir.path().join("bundle.py");
    let output = Command::new(env!("CARGO_BIN_EXE_cribo"))
        .args([
            "--entry",
            &entry_arg(&dir),
            "--output",
            &out.to_string_lossy(),
        ])
        .env("CRIBO_SOURCEMAP", "external")
        .env("CRIBO_SOURCES_CONTENT", "false")
        .output()
        .expect("run cribo binary");
    assert!(
        output.status.success(),
        "bundling must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map"))
        .expect("CRIBO_SOURCEMAP env var must enable map emission");
    assert_map_covers_helper(&map_json, "bundle.py");
    assert!(
        !map_json.contains("sourcesContent"),
        "CRIBO_SOURCES_CONTENT=false must strip embedding"
    );
}

#[test]
fn invalid_sourcemap_env_values_warn_and_are_ignored() {
    let dir = fixture_project();
    let out = dir.path().join("bundle.py");
    let output = Command::new(env!("CARGO_BIN_EXE_cribo"))
        .args([
            "--entry",
            &entry_arg(&dir),
            "--output",
            &out.to_string_lossy(),
        ])
        .env("CRIBO_SOURCEMAP", "lnked")
        .env("CRIBO_SOURCES_CONTENT", "sometimes")
        .output()
        .expect("run cribo binary");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "bundling must succeed: {stderr}");
    assert!(
        stderr.contains("Ignoring CRIBO_SOURCEMAP='lnked'"),
        "invalid sourcemap mode must be reported: {stderr}"
    );
    assert!(
        stderr.contains("Ignoring CRIBO_SOURCES_CONTENT='sometimes'"),
        "invalid sources-content value must be reported: {stderr}"
    );
    assert!(
        !dir.path().join("bundle.py.map").exists(),
        "invalid environment values must leave source maps disabled"
    );
}

#[test]
fn runtime_keeps_thread_sys_exit_silent() {
    let dir = make_project(&[
        (
            "main.py",
            "import sys\nimport threading\n\nworker = threading.Thread(target=lambda: \
             sys.exit(3))\nworker.start()\nworker.join()\nprint(\"done\")\n",
        ),
        ("helper.py", "unused = True\n"),
    ]);
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let (ok, stdout, stderr) = run_python(&out, &[]);
    assert!(
        ok,
        "sys.exit in a worker thread must not fail the process: {stderr}"
    );
    assert!(stdout.contains("done"));
    assert!(
        stderr.is_empty(),
        "SystemExit in a thread must stay silent, as with the default hook: {stderr}"
    );
}

#[test]
fn runtime_notifies_preinstalled_custom_excepthook() {
    // A custom excepthook installed before the bundle's prologue (via
    // sitecustomize) must still observe the exception after a successful remap.
    let dir = crash_project();
    fs::write(
        dir.path().join("sitecustomize.py"),
        "import sys\n\n_original = sys.excepthook\n\n\ndef reporting_hook(exc_type, exc_value, \
         tb):\n    print(\"REPORTER SAW:\", exc_type.__name__, file=sys.stderr)\n\n\nsys.excepthook \
         = reporting_hook\n",
    )
    .expect("write sitecustomize");
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");

    let mut command = Command::new(common::get_python_executable());
    command.arg(&bundle);
    command.env_remove("CRIBO_SOURCE_MAPS");
    // Make usercustomize importable so the custom hook installs before the bundle.
    command.env("PYTHONPATH", dir.path());
    let output = command.output().expect("run python");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert_remapped(&stderr);
    assert!(
        stderr.contains("REPORTER SAW: ValueError"),
        "the preinstalled custom hook must still be notified after a remap: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Second review round: shadowing, docstring, tracebacklimit, notes, handlers
// ---------------------------------------------------------------------------

#[test]
fn runtime_survives_shadowing_threading_module() {
    // A project file named threading.py next to the bundle must not be able
    // to break (or be imported by) the runtime's own stdlib imports.
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    fs::write(
        dir.path().join("threading.py"),
        "raise RuntimeError(\"shadow module imported\")\n",
    )
    .expect("write shadowing module");

    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok, "the crash must still surface");
    assert!(
        !stderr.contains("shadow module imported"),
        "the runtime must not import the adjacent threading.py: {stderr}"
    );
    assert_remapped(&stderr);
}

#[test]
fn bundle_docstring_survives_runtime_injection() {
    let dir = make_project(&[
        ("main.py", "\"\"\"Entry doc.\"\"\"\n\nprint(__doc__)\n"),
        ("helper.py", "unused = True\n"),
    ]);
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=inline",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let (ok, stdout, stderr) = run_python(&out, &[]);
    assert!(ok, "bundle must run: {stderr}");
    assert!(
        stdout.contains("Entry doc."),
        "the injected runtime must not displace the bundle docstring: {stdout}"
    );
}

#[test]
fn runtime_honors_tracebacklimit() {
    let dir = make_project(&[
        (
            "main.py",
            // Set the limit on the real sys module: the bundler's stdlib
            // import proxy forwards attribute reads but not writes, so a
            // plain `sys.tracebacklimit = 0` would only decorate the proxy
            // (true for bundles with or without source maps).
            "from helper import boom\n\n__import__(\"sys\").tracebacklimit = 0\nboom()\n",
        ),
        (
            "helper.py",
            "def boom():\n    raise ValueError(\"limited kaboom\")\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(stderr.contains("ValueError: limited kaboom"), "{stderr}");
    assert!(
        !stderr.contains("Traceback (most recent call last):"),
        "tracebacklimit = 0 must suppress the frame listing: {stderr}"
    );
    assert!(
        !stderr.contains("File \""),
        "tracebacklimit = 0 must suppress all frames: {stderr}"
    );
}

#[test]
fn runtime_renders_exception_notes() {
    if !python_at_least(11) {
        return;
    }
    let dir = make_project(&[
        ("main.py", "from helper import boom\n\nboom()\n"),
        (
            "helper.py",
            "def boom():\n    error = ValueError(\"kaboom\")\n    error.add_note(\"NOTE: check \
             the flux capacitor\")\n    raise error\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("helper.py\", line 4, in boom"),
        "traceback must be remapped: {stderr}"
    );
    assert!(
        stderr.contains("NOTE: check the flux capacitor"),
        "__notes__ must survive remapped rendering: {stderr}"
    );
}

#[test]
fn map_covers_elif_and_except_headers() {
    let dir = make_project(&[
        (
            "main.py",
            "from helper import classify\n\nprint(classify(2))\n",
        ),
        (
            "helper.py",
            "def classify(value):\n    if value == 0:\n        return \"zero\"\n    elif value == \
             1:\n        return \"one\"\n    elif value == 2:\n        return \"two\"\n    \
             try:\n        return int(value)\n    except ValueError:\n        return \"other\"\n",
        ),
    ]);
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map");
    let map = parse_map(&map_json);

    let helper_id = (0..map.get_sources().count() as u32)
        .find(|id| {
            map.get_source(*id)
                .is_some_and(|s| s.ends_with("helper.py"))
        })
        .expect("helper.py in sources");
    let mapped_helper_lines: Vec<u32> = map
        .get_tokens()
        .filter(|token| token.get_source_id() == Some(helper_id))
        .map(|token| token.get_src_line())
        .collect();
    // 0-based original lines: 3 and 5 are the `elif` headers, 9 is `except ValueError:`.
    for header_line in [3, 5, 9] {
        assert!(
            mapped_helper_lines.contains(&header_line),
            "helper.py 0-based line {header_line} (a clause header) must be mapped; mapped \
             lines: {mapped_helper_lines:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Third review round: specialized formatting, long chains, header coverage
// ---------------------------------------------------------------------------

#[test]
fn runtime_keeps_name_error_suggestions() {
    if !python_at_least(10) {
        return;
    }
    let dir = make_project(&[
        ("main.py", "from helper import go\n\ngo()\n"),
        (
            "helper.py",
            "def go():\n    valuable = 1\n    return valuabl\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("helper.py\", line 3, in go"),
        "traceback must be remapped: {stderr}"
    );
    assert!(stderr.contains("NameError"), "{stderr}");
    assert!(
        stderr.contains("Did you mean"),
        "interpreter suggestions must survive remapped rendering: {stderr}"
    );
}

#[test]
fn runtime_renders_long_exception_chains_fully() {
    // 20 chained causes exceed the previous traversal cap; the innermost
    // (root) exception and its traceback must still be rendered.
    let dir = make_project(&[
        ("main.py", "from helper import cascade\n\ncascade()\n"),
        (
            "helper.py",
            "def cascade():\n    try:\n        raise ValueError(\"root kaboom\")\n    except \
             ValueError as error:\n        current = error\n        for depth in range(20):\n            \
             try:\n                raise RuntimeError(\"layer %d\" % depth) from \
             current\n            except RuntimeError as next_error:\n                current = \
             next_error\n        raise current\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("ValueError: root kaboom"),
        "the root cause of a 20-deep chain must be rendered: {stderr}"
    );
    assert!(stderr.contains("layer 19"), "{stderr}");
    assert!(
        stderr.contains("helper.py\", line 3"),
        "the root cause frame must be remapped: {stderr}"
    );
}

#[test]
fn map_covers_match_case_headers_and_decorators() {
    let dir = make_project(&[
        (
            "main.py",
            "from helper import Widget, run\n\nprint(run(1))\nprint(Widget())\n",
        ),
        (
            "helper.py",
            "def trace(func):\n    return func\n\n\n@trace\n@trace\ndef run(value):\n    match \
             value:\n        case 0:\n            return \"zero\"\n        case _ if value > \
             0:\n            return \"positive\"\n        case _:\n            return \
             \"negative\"\n\n\n@trace\nclass Widget(dict):\n    pass\n",
        ),
    ]);
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map");
    let map = parse_map(&map_json);

    let helper_id = (0..map.get_sources().count() as u32)
        .find(|id| {
            map.get_source(*id)
                .is_some_and(|s| s.ends_with("helper.py"))
        })
        .expect("helper.py in sources");
    let mapped_helper_lines: Vec<u32> = map
        .get_tokens()
        .filter(|token| token.get_source_id() == Some(helper_id))
        .map(|token| token.get_src_line())
        .collect();
    // 0-based original lines: 4 and 5 are the def decorators; 6 is the
    // decorated `def run(value):` header itself; 8, 10, and 12 are the `case`
    // headers; 16 and 17 are the decorated class's decorator and header (the
    // inliner preserves the original name range on renamed identifiers, and
    // the base-class list serves as a fallback anchor).
    for header_line in [4, 5, 6, 8, 10, 12, 16, 17] {
        assert!(
            mapped_helper_lines.contains(&header_line),
            "helper.py 0-based line {header_line} (decorator or case header) must be mapped; \
             mapped lines: {mapped_helper_lines:?}"
        );
    }
}

#[test]
fn runtime_survives_shadowed_builtins() {
    // Entry code that rebinds common builtins at module level must not break
    // the runtime: every method snapshots its builtins at definition time.
    let dir = make_project(&[
        (
            "main.py",
            "from helper import boom\n\nopen = None\nlen = None\nmax = None\nset = \
             None\ngetattr = None\nboom()\n",
        ),
        (
            "helper.py",
            "def boom():\n    inner()\n\ndef inner():\n    raise ValueError(\"kaboom\")\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("helper.py\", line 5, in inner"),
        "traceback must be remapped despite shadowed builtins: {stderr}"
    );
    assert!(
        stderr.contains("main.py\", line 8, in <module>"),
        "entry frame must be remapped despite shadowed builtins: {stderr}"
    );
    assert!(!stderr.contains("bundle.py\", line"), "{stderr}");
}

/// A relative bundle path remains anchored after user code changes directory.
#[test]
fn runtime_survives_chdir_before_crash() {
    // A bundle launched via a relative path that chdirs away before crashing
    // must still locate itself and its sibling map (paths are anchored at
    // startup).
    let dir = make_project(&[
        (
            "main.py",
            "import os\nfrom helper import boom\n\nos.chdir(\"/\")\nboom()\n",
        ),
        (
            "helper.py",
            "def boom():\n    raise ValueError(\"kaboom after chdir\")\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");

    let mut command = Command::new(common::get_python_executable());
    // Invoke through the relative file name, from the bundle's directory.
    command.arg(bundle.file_name().expect("bundle name"));
    command.current_dir(dir.path());
    command.env_remove("CRIBO_SOURCE_MAPS");
    let output = command.output().expect("run python");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("helper.py\", line 2, in boom"),
        "remapping must survive os.chdir before the crash: {stderr}"
    );
    assert!(stderr.contains("kaboom after chdir"));
}

/// Stacked source-map runtimes render one traceback instead of chaining printers.
#[test]
fn stacked_runtimes_do_not_duplicate_tracebacks() {
    // Two source-mapped bundles executed in one interpreter stack their hooks;
    // the outer runtime must recognize the inner one and not chain into it
    // (which would fall through to the default printer and duplicate output).
    let quiet = fixture_project();
    let quiet_bundle = quiet.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&quiet),
        "--output",
        &quiet_bundle.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");

    let crash = crash_project();
    let crash_bundle = bundle_crash_project(&crash, "--sourcemap=linked");

    let driver = crash.path().join("driver.py");
    fs::write(
        &driver,
        format!(
            "import runpy\nrunpy.run_path({quiet:?})\nrunpy.run_path({crash:?})\n",
            quiet = quiet_bundle.to_string_lossy(),
            crash = crash_bundle.to_string_lossy(),
        ),
    )
    .expect("write driver");

    let (ok, _, stderr) = run_python(&driver, &[]);
    assert!(!ok);
    assert_eq!(
        stderr.matches("Traceback (most recent call last):").count(),
        1,
        "stacked runtimes must render exactly one traceback: {stderr}"
    );
    assert_eq!(
        stderr.matches("ValueError: kaboom").count(),
        1,
        "the exception line must print exactly once: {stderr}"
    );
    assert!(
        stderr.contains("helper.py\", line 5, in inner"),
        "the crashing bundle's frames must be remapped: {stderr}"
    );
}

#[test]
fn runtime_remaps_when_group_is_suppressed() {
    if !python_at_least(11) {
        return;
    }
    // A caught ExceptionGroup replaced via `raise ... from None` never
    // renders; its hidden presence in __context__ must not force the
    // unremapped fallback for the visible ordinary exception.
    let dir = make_project(&[
        ("main.py", "from helper import convert\n\nconvert()\n"),
        (
            "helper.py",
            "def convert():\n    try:\n        raise ExceptionGroup(\"grp\", \
             [ValueError(\"inner\")])\n    except ExceptionGroup:\n        raise \
             RuntimeError(\"converted kaboom\") from None\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("RuntimeError: converted kaboom"),
        "{stderr}"
    );
    assert!(
        stderr.contains("helper.py\", line 5, in convert"),
        "a suppressed group in __context__ must not disable remapping: {stderr}"
    );
    assert!(
        !stderr.contains("ExceptionGroup"),
        "the suppressed group must not be rendered: {stderr}"
    );
}

#[test]
fn runtime_rejects_mismatched_sibling_map() {
    // Linked bundles embed a SHA-256 of their map; a sibling map from a
    // different build (concurrent-build interleaving, manual copying) must be
    // ignored rather than silently applying wrong mappings.
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");

    // Build a different project and steal its map.
    let other = make_project(&[
        ("main.py", "from helper import other\n\nprint(other())\n"),
        ("helper.py", "def other():\n    return 42\n"),
    ]);
    let other_bundle = other.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&other),
        "--output",
        &other_bundle.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    fs::copy(
        other.path().join("bundle.py.map"),
        dir.path().join("bundle.py.map"),
    )
    .expect("swap in a foreign map");

    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert_standard_traceback(&stderr);
}

#[cfg(unix)]
#[test]
fn linked_comment_escapes_control_characters_in_names() {
    // A newline inside the output file name must not break out of the
    // sourceMappingURL comment (which would inject executable text).
    let dir = fixture_project();
    let out = dir.path().join("bun\ndle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(
        bundle.contains("# sourceMappingURL=bun%0Adle.py.map"),
        "control characters in the map name must be percent-encoded"
    );
    // The bundle must remain valid, runnable Python.
    let (ok, stdout, stderr) = run_python(&out, &[]);
    assert!(ok, "bundle must run: {stderr}");
    assert!(stdout.contains("hello world"));
}

#[test]
fn linked_comment_preserves_unicode_names() {
    // Non-ASCII characters in the output name must pass through verbatim
    // (percent-encoding is reserved for control characters), so external map
    // consumers following the comment can locate the sibling file.
    let dir = fixture_project();
    let out = dir.path().join("bündle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(
        bundle.contains("# sourceMappingURL=bündle.py.map"),
        "unicode map names must not be mangled"
    );
    let (ok, stdout, stderr) = run_python(&out, &[]);
    assert!(ok, "bundle must run: {stderr}");
    assert!(stdout.contains("hello world"));
}

#[test]
fn external_mode_rejects_mismatched_sibling_map() {
    // External mode embeds the same digest as linked mode; CRIBO_SOURCE_MAPS=1
    // against a foreign sibling map must fall back to the standard traceback.
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=external");

    let other = make_project(&[
        ("main.py", "from helper import other\n\nprint(other())\n"),
        ("helper.py", "def other():\n    return 42\n"),
    ]);
    let other_bundle = other.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&other),
        "--output",
        &other_bundle.to_string_lossy(),
        "--sourcemap=external",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    fs::copy(
        other.path().join("bundle.py.map"),
        dir.path().join("bundle.py.map"),
    )
    .expect("swap in a foreign map");

    let (ok, _, stderr) = run_python(&bundle, &[("CRIBO_SOURCE_MAPS", "1")]);
    assert!(!ok);
    assert_standard_traceback(&stderr);
}

#[test]
fn stacked_runtimes_merge_maps_across_bundles() {
    // A traceback crossing two source-mapped bundles (the crashing bundle
    // calls a function retained from an earlier one) must remap the frames of
    // BOTH bundles; unmapped driver frames keep standard rendering.
    let provider = make_project(&[
        (
            "main.py",
            // The real builtins module (bypassing the bundler's stdlib proxy,
            // whose speculative import_module would add chained-context noise).
            "from helper import provider_boom\n\n__import__(\"builtins\").provider_boom = \
             provider_boom\nprint(\"provider ready\")\n",
        ),
        (
            "helper.py",
            "def provider_boom():\n    raise ValueError(\"cross-bundle kaboom\")\n",
        ),
    ]);
    let provider_bundle = provider.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&provider),
        "--output",
        &provider_bundle.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");

    let caller = make_project(&[
        (
            "main.py",
            "print(\"calling provider\")\n__import__(\"builtins\").provider_boom()\n",
        ),
        ("helper.py", "unused = True\n"),
    ]);
    let caller_bundle = caller.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&caller),
        "--output",
        &caller_bundle.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");

    let driver = caller.path().join("driver.py");
    fs::write(
        &driver,
        format!(
            "import runpy\nrunpy.run_path({provider:?})\nrunpy.run_path({caller:?})\n",
            provider = provider_bundle.to_string_lossy(),
            caller = caller_bundle.to_string_lossy(),
        ),
    )
    .expect("write driver");

    let (ok, _, stderr) = run_python(&driver, &[]);
    assert!(!ok);
    assert_eq!(
        stderr.matches("Traceback (most recent call last):").count(),
        1,
        "exactly one traceback expected: {stderr}"
    );
    // The caller bundle's frame (installed second, renders) is remapped...
    assert!(
        stderr.contains("main.py\", line 2, in <module>"),
        "the caller frame must be remapped: {stderr}"
    );
    // ...and so is the provider bundle's frame, via the runtime registry.
    assert!(
        stderr.contains("helper.py\", line 2, in provider_boom"),
        "the earlier bundle's frame must be remapped through the shared registry: {stderr}"
    );
    assert!(stderr.contains("cross-bundle kaboom"));
}

#[test]
fn runtime_refuses_map_after_bundle_replacement() {
    // A bundle rebuilt at the same path while an old instance is running must
    // not have its (new) map applied to the old in-memory code. The digest of
    // this build's map is baked into the executing code, so the old process
    // rejects the replacement map. Simulated here by rebuilding with shifted
    // lines and running the ORIGINAL bundle against the REBUILT map.
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let original_bundle = fs::read(&bundle).expect("read original bundle");

    // Rebuild with different line numbers → different map (and digest).
    fs::write(
        dir.path().join("helper.py"),
        "# shifted\n# shifted again\ndef boom():\n    inner()\n\ndef inner():\n    raise \
         ValueError(\"kaboom\")\n",
    )
    .expect("shift helper lines");
    bundle_crash_project(&dir, "--sourcemap=linked");

    // Old bundle + new map: the replacement pairing must be refused.
    fs::write(&bundle, original_bundle).expect("restore original bundle");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert_standard_traceback(&stderr);
}

#[test]
fn runtime_survives_shadowed_threading_under_runpy() {
    // Under runpy, sys.path[0] is the DRIVER's directory; the bundle's own
    // directory is added only after importing runpy, then must be filtered when
    // the runtime imports its stdlib dependencies.
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    fs::write(
        dir.path().join("threading.py"),
        "raise RuntimeError(\"shadow module imported\")\n",
    )
    .expect("write shadowing module");

    let driver_dir = TempDir::new().expect("create driver dir");
    let driver = driver_dir.path().join("driver.py");
    fs::write(
        &driver,
        format!(
            "import runpy\nimport sys\nsys.path.insert(0, {bundle_dir:?})\nrunpy.run_path({bundle:?}, \
             run_name=\"__main__\")\n",
            bundle_dir = dir.path().to_string_lossy(),
            bundle = bundle.to_string_lossy(),
        ),
    )
    .expect("write driver");

    let mut command = Command::new(common::get_python_executable());
    command.arg(&driver);
    command.env_remove("CRIBO_SOURCE_MAPS");
    let output = command.output().expect("run python");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        !stderr.contains("shadow module imported"),
        "the runtime must not import the bundle-adjacent threading.py: {stderr}"
    );
    assert!(
        stderr.contains("helper.py\", line 5, in inner"),
        "remapping must work under runpy: {stderr}"
    );
}

#[test]
fn stacked_runtimes_still_notify_preinstalled_custom_hook() {
    // A custom hook installed before ANY bundle must still observe exceptions
    // when several cribo runtimes have stacked on top of it: the notifier
    // traverses through earlier cribo hooks to the original custom one.
    let quiet = fixture_project();
    let quiet_bundle = quiet.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&quiet),
        "--output",
        &quiet_bundle.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let crash = crash_project();
    let crash_bundle = bundle_crash_project(&crash, "--sourcemap=linked");

    let driver_dir = TempDir::new().expect("create driver dir");
    fs::write(
        driver_dir.path().join("sitecustomize.py"),
        "import sys\n\n\ndef reporting_hook(exc_type, exc_value, tb):\n    print(\"REPORTER \
         SAW:\", exc_type.__name__, file=sys.stderr)\n\n\nsys.excepthook = reporting_hook\n",
    )
    .expect("write sitecustomize");
    let driver = driver_dir.path().join("driver.py");
    fs::write(
        &driver,
        format!(
            "import runpy\nrunpy.run_path({quiet:?})\nrunpy.run_path({crash:?})\n",
            quiet = quiet_bundle.to_string_lossy(),
            crash = crash_bundle.to_string_lossy(),
        ),
    )
    .expect("write driver");

    let mut command = Command::new(common::get_python_executable());
    command.arg(&driver);
    command.env_remove("CRIBO_SOURCE_MAPS");
    command.env("PYTHONPATH", driver_dir.path());
    let output = command.output().expect("run python");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("helper.py\", line 5, in inner"),
        "remapping must work: {stderr}"
    );
    assert!(
        stderr.contains("REPORTER SAW: ValueError"),
        "the custom hook below two stacked runtimes must still be notified: {stderr}"
    );
    assert_eq!(
        stderr.matches("Traceback (most recent call last):").count(),
        1,
        "no duplicate rendering through the default printer: {stderr}"
    );
}

#[test]
fn runtime_remaps_raise_inside_multiline_fstring() {
    // ruff's generator emits multiline f-strings on a single physical line
    // (`\n` escapes), so a replacement expression raising on what was an
    // interior line in the ORIGINAL source is attributed to the statement's
    // single generated line — which must remap to the statement's original
    // starting line.
    let dir = make_project(&[
        ("main.py", "from helper import render\n\nprint(render(0))\n"),
        (
            "helper.py",
            "def render(value):\n    banner = f\"\"\"first {value}\nsecond {1 // value}\nthird \
             {value}\"\"\"\n    return banner\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(stderr.contains("ZeroDivisionError"), "{stderr}");
    assert!(
        stderr.contains("helper.py\", line 2, in render"),
        "a raise inside a multiline f-string must map to the statement's original start line: \
         {stderr}"
    );
}

#[test]
fn sources_with_url_delimiters_are_encoded_and_decoded() {
    // '#' in a directory name would be read as a URL fragment by map
    // consumers; the map must percent-encode it and the runtime must decode
    // it back before touching the filesystem.
    let dir = TempDir::new().expect("create temp dir");
    let src_dir = dir.path().join("we#ird");
    fs::create_dir_all(&src_dir).expect("create source dir");
    fs::write(
        src_dir.join("main.py"),
        "from helper import boom\n\nboom()\n",
    )
    .expect("write main.py");
    fs::write(
        src_dir.join("helper.py"),
        "def boom():\n    raise ValueError(\"kaboom\")\n",
    )
    .expect("write helper.py");

    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &src_dir.join("main.py").to_string_lossy(),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map");
    assert!(
        map_json.contains("we%23ird"),
        "URL delimiters in source paths must be percent-encoded: {map_json}"
    );

    let (ok, _, stderr) = run_python(&out, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("we#ird/helper.py\", line 2, in boom")
            || stderr.contains("we#ird\\helper.py\", line 2, in boom"),
        "the runtime must decode the path before display and file access: {stderr}"
    );
    assert!(
        stderr.contains("raise ValueError(\"kaboom\")"),
        "the original source line must load from the decoded path: {stderr}"
    );
}

#[test]
fn digest_placeholder_in_user_code_is_left_untouched() {
    // Only the prologue's own placeholder receives the digest; a user string
    // that happens to share the spelling must survive verbatim, and the
    // runtime must still verify (i.e. the prologue occurrence was the one
    // substituted).
    let dir = make_project(&[
        (
            "main.py",
            "from helper import boom\n\nprint(\"__CRIBO_SOURCEMAP_DIGEST__\")\nboom()\n",
        ),
        (
            "helper.py",
            "def boom():\n    raise ValueError(\"kaboom\")\n",
        ),
    ]);
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert_eq!(
        bundle.matches("__CRIBO_SOURCEMAP_DIGEST__").count(),
        1,
        "exactly the user occurrence must remain (prologue one substituted)"
    );

    let (ok, stdout, stderr) = run_python(&out, &[]);
    assert!(!ok);
    assert!(
        stdout.contains("__CRIBO_SOURCEMAP_DIGEST__"),
        "user string must print unchanged: {stdout}"
    );
    // Remapping proves the prologue digest matched the sibling map.
    assert!(
        stderr.contains("helper.py\", line 2, in boom"),
        "traceback must remap, proving digest verification passed: {stderr}"
    );
    assert!(stderr.contains("raise ValueError(\"kaboom\")"), "{stderr}");
}

#[cfg(target_os = "linux")]
#[test]
fn linked_comment_encodes_non_utf8_names() {
    // Unix filenames need not be UTF-8. A lossy conversion would bake U+FFFD
    // into the sourceMappingURL comment — a name that does not exist on disk.
    // The raw invalid byte must be percent-encoded instead.
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let dir = fixture_project();
    let mut name = b"bun".to_vec();
    name.push(0xFF);
    name.extend_from_slice(b"dle.py");
    let out = dir.path().join(OsString::from_vec(name));
    let output = Command::new(env!("CARGO_BIN_EXE_cribo"))
        .arg("--entry")
        .arg(dir.path().join("main.py"))
        .arg("--output")
        .arg(&out)
        .arg("--sourcemap=linked")
        .output()
        .expect("run cribo");
    assert!(
        output.status.success(),
        "bundling must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(
        bundle.contains("# sourceMappingURL=bun%FFdle.py.map"),
        "invalid bytes must be percent-encoded, not replaced with U+FFFD"
    );
    assert!(
        !bundle.contains('\u{FFFD}'),
        "no lossy replacement character may reach the comment"
    );
    // The bundle must remain valid, runnable Python (with its sibling map).
    let (ok, stdout, stderr) = run_python(&out, &[]);
    assert!(ok, "bundle must run: {stderr}");
    assert!(stdout.contains("hello world"));
}

#[test]
fn bundle_namespace_is_clean_of_runtime_helpers() {
    // The prologue must leave no helper names behind: user code and
    // `from bundle import *` consumers must not see (or collide with) them.
    let dir = make_project(&[(
        "main.py",
        "prologue_names = (\"_cribo_sys\", \"_cribo_sm_import\", \"_CriboSmStream\", \"_CriboSourceMapRuntime\")\nleaked = sorted(name for name in prologue_names if name in globals())\nprint(\"LEAKED:\", leaked)\n",
    )]);
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=inline",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let (ok, stdout, stderr) = run_python(&out, &[]);
    assert!(ok, "bundle must run: {stderr}");
    assert!(
        stdout.contains("LEAKED: []"),
        "runtime helper names must not leak into the bundle namespace: {stdout}"
    );
}

#[test]
fn map_covers_decorated_class_without_argument_list() {
    // `class Widget:` has no base-class list to fall back on, and the inliner
    // rewrites every inlined class name — the header anchor must survive via
    // the preserved identifier range, or exceptions CPython attributes to the
    // class header during construction (e.g. a descriptor raising from
    // `__set_name__`) stay on bundle coordinates.
    let dir = make_project(&[
        (
            "main.py",
            "from helper import Widget\n\nprint(Widget().value)\n",
        ),
        (
            "helper.py",
            "def trace(cls):\n    return cls\n\n\n@trace\nclass Widget:\n    def \
             __init__(self):\n        self.value = \"widget value\"\n",
        ),
    ]);
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map");
    let map = parse_map(&map_json);

    let helper_id = (0..map.get_sources().count() as u32)
        .find(|id| {
            map.get_source(*id)
                .is_some_and(|s| s.ends_with("helper.py"))
        })
        .expect("helper.py in sources");
    let mapped_helper_lines: Vec<u32> = map
        .get_tokens()
        .filter(|token| token.get_source_id() == Some(helper_id))
        .map(|token| token.get_src_line())
        .collect();
    // 0-based: 4 is the `@trace` decorator, 5 the bare `class Widget:` header.
    for header_line in [4, 5] {
        assert!(
            mapped_helper_lines.contains(&header_line),
            "helper.py 0-based line {header_line} (decorator or bare class header) must be \
             mapped; mapped lines: {mapped_helper_lines:?}"
        );
    }
}

#[test]
fn truncated_traceback_keeps_pep657_on_unmapped_frames() {
    // With a positive sys.tracebacklimit the runtime keeps the LAST n frames
    // (like the interpreter's C printer), while the traceback module keeps
    // the FIRST n. The PEP 657 summaries must therefore be extracted
    // untruncated, or a retained unmapped frame would index past (or into the
    // wrong slot of) the shortened list and lose its caret anchors.
    let dir = make_project(&[
        (
            "main.py",
            "import importlib.util\nimport os\n\nfrom helper import boom\n\n__import__(\"sys\")\
             .tracebacklimit = 2\nspec = importlib.util.spec_from_file_location(\"ext\", \
             os.path.join(os.path.dirname(os.path.abspath(__file__)), \"ext.py\"))\next = \
             importlib.util.module_from_spec(spec)\nspec.loader.exec_module(ext)\nboom(ext)\n",
        ),
        ("helper.py", "def boom(ext):\n    ext.go()\n"),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    // Placed next to the bundle and loaded by explicit path at run time, so
    // cribo never bundles it: its frames stay unmapped by design.
    fs::write(
        dir.path().join("ext.py"),
        "def go():\n    left = 1\n    return left + \"boom\"\n",
    )
    .expect("write ext.py");

    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    // Retained frames: helper.boom (remapped) and ext.go (unmapped).
    assert!(
        stderr.contains("helper.py\", line 2, in boom"),
        "the retained bundle frame must remap: {stderr}"
    );
    assert!(
        stderr.contains("ext.py\", line 3, in go"),
        "the retained external frame must render: {stderr}"
    );
    assert!(
        !stderr.contains("main.py"),
        "tracebacklimit = 2 must drop the module frame: {stderr}"
    );
    // The caret line proves the frame came from the standard summaries (the
    // plain fallback prints only the file line and source text).
    assert!(
        stderr.contains('^'),
        "the unmapped frame must keep its PEP 657 carets: {stderr}"
    );
}

#[test]
fn bootstrap_survives_shadowed_transitive_stdlib_imports() {
    // The runtime's own imports resolve through a filtered path snapshot, but
    // modules imported *while those stdlib modules execute* (e.g. traceback
    // importing textwrap) must be protected too: an adjacent project file
    // named like a stdlib dependency must neither execute during bootstrap
    // nor disable remapping by raising.
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    fs::write(
        dir.path().join("textwrap.py"),
        "print(\"SHADOWED TEXTWRAP EXECUTED\")\nraise RuntimeError(\"shadowed textwrap\")\n",
    )
    .expect("write shadowing textwrap.py");

    let (ok, stdout, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(
        !stdout.contains("SHADOWED TEXTWRAP EXECUTED"),
        "the adjacent textwrap.py must never execute during bootstrap: {stdout}"
    );
    assert_remapped(&stderr);
}

#[cfg(unix)]
#[test]
fn sources_with_backslashes_are_encoded_and_decoded() {
    // On Unix a backslash is a legal filename character but not a valid
    // unescaped URL-path character; `sources` entries must carry %5C and the
    // runtime must restore the filesystem spelling before file access.
    let dir = TempDir::new().expect("create temp dir");
    let src_dir = dir.path().join("we\\ird");
    fs::create_dir_all(&src_dir).expect("create source dir");
    fs::write(
        src_dir.join("main.py"),
        "from helper import boom\n\nboom()\n",
    )
    .expect("write main.py");
    fs::write(
        src_dir.join("helper.py"),
        "def boom():\n    raise ValueError(\"kaboom\")\n",
    )
    .expect("write helper.py");

    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &src_dir.join("main.py").to_string_lossy(),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map");
    assert!(
        map_json.contains("we%5Cird"),
        "backslashes in source paths must be percent-encoded: {map_json}"
    );

    let (ok, _, stderr) = run_python(&out, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("we\\ird/helper.py\", line 2, in boom"),
        "the runtime must decode %5C back to a backslash for display and file access: {stderr}"
    );
    assert!(
        stderr.contains("raise ValueError(\"kaboom\")"),
        "the original source line must load from the decoded path: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn runtime_resolves_symlinked_bundle_to_sibling_map() {
    // A bundle launched through a deployment symlink keeps the symlink
    // spelling in __file__; the sibling map lives next to the real file and
    // must still be found.
    let dir = crash_project();
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let link_dir = TempDir::new().expect("create link dir");
    let link = link_dir.path().join("app");
    std::os::unix::fs::symlink(&bundle, &link).expect("create symlink");

    let (ok, _, stderr) = run_python(&link, &[]);
    assert!(!ok);
    assert_remapped(&stderr);
}

#[cfg(unix)]
#[test]
fn runtime_resolves_sources_from_symlinked_output_directory() {
    // The resolver may canonicalize source modules while the CLI output keeps
    // a symlinked directory spelling. Map relativization must canonicalize both
    // sides so resolving the real bundle path does not duplicate path prefixes.
    let project = crash_project();
    let link_parent = TempDir::new().expect("create link parent");
    let linked_project = link_parent.path().join("project");
    std::os::unix::fs::symlink(project.path(), &linked_project).expect("create project symlink");

    let bundle = linked_project.join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &linked_project.join("main.py").to_string_lossy(),
        "--output",
        &bundle.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");

    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert_remapped(&stderr);
}

#[test]
fn rebuilt_bundle_at_same_path_declines_stale_remap() {
    // Bundle A stays imported while its file is rebuilt in place as bundle B;
    // a traceback through A's retained code objects shares B's filename
    // spelling and anchor, but B's mappings do not describe A's code. The
    // runtimes' build digests differ, so the ambiguity check must decline
    // rather than apply B's map to A's frames.
    let dir = make_project(&[
        (
            "main_v1.py",
            "def boom():\n    raise ValueError(\"kaboom v1\")\n\n\nassert callable(boom)\n",
        ),
        (
            "main_v2.py",
            "# filler line so v2 maps differently\n# more filler\n\n\ndef boom():\n    raise \
             ValueError(\"kaboom v2\")\n\n\nassert callable(boom)\n",
        ),
        (
            "driver.py",
            "import importlib.util\nimport os\nimport shutil\nimport sys\n\n# The rebuild \
             happens within pyc mtime granularity; a stale cache would\n# silently re-execute \
             v1 and mask the scenario.\nsys.dont_write_bytecode = True\n\nbase = \
             os.path.dirname(os.path.abspath(__file__))\ntarget = os.path.join(base, \
             \"app.py\")\n\n\ndef load(tag):\n    spec = \
             importlib.util.spec_from_file_location(\"app_\" + tag, target)\n    module = \
             importlib.util.module_from_spec(spec)\n    spec.loader.exec_module(module)\n    \
             return module\n\n\nshutil.copyfile(os.path.join(base, \"app_v1.py\"), \
             target)\nshutil.copyfile(os.path.join(base, \"app_v1.py.map\"), target + \
             \".map\")\nold = load(\"v1\")\nshutil.copyfile(os.path.join(base, \
             \"app_v2.py\"), target)\nshutil.copyfile(os.path.join(base, \
             \"app_v2.py.map\"), target + \".map\")\nload(\"v2\")\nold.boom()\n",
        ),
    ]);
    for version in ["v1", "v2"] {
        let (ok, _, stderr) = run_cribo(&[
            "--entry",
            &dir.path()
                .join(format!("main_{version}.py"))
                .to_string_lossy(),
            "--output",
            &dir.path()
                .join(format!("app_{version}.py"))
                .to_string_lossy(),
            "--sourcemap=linked",
        ]);
        assert!(ok, "bundling {version} must succeed: {stderr}");
    }

    let (ok, _, stderr) = run_python(&dir.path().join("driver.py"), &[]);
    assert!(!ok);
    assert!(
        stderr.contains("kaboom v1"),
        "the retained v1 function must raise: {stderr}"
    );
    assert!(
        !stderr.contains("main_v1.py") && !stderr.contains("main_v2.py"),
        "neither build's mappings may be applied to ambiguous same-path frames: {stderr}"
    );
    assert!(
        stderr.contains("app.py\", line"),
        "the declined traceback must stay on bundle coordinates: {stderr}"
    );
    assert_eq!(
        stderr.matches("Traceback (most recent call last):").count(),
        1,
        "exactly one traceback must print: {stderr}"
    );
}

#[test]
fn digest_survives_entry_docstring_with_placeholder() {
    // The entry docstring can precede the injected prologue in the emitted
    // bundle, so a docstring containing the placeholder spelling is the first
    // textual match. The substitution is anchored on the bootstrap line: the
    // docstring (and thus the program's __doc__) must stay untouched and
    // digest verification must still pass.
    let dir = make_project(&[(
        "main.py",
        "\"\"\"Docs mention __CRIBO_SOURCEMAP_DIGEST__ here.\"\"\"\n\nprint(repr(__doc__))\
         \nraise ValueError(\"kaboom\")\n",
    )]);
    let out = dir.path().join("bundle.py");
    let (ok, _, stderr) = run_cribo(&[
        "--entry",
        &entry_arg(&dir),
        "--output",
        &out.to_string_lossy(),
        "--sourcemap=linked",
    ]);
    assert!(ok, "bundling must succeed: {stderr}");

    let (ok, stdout, stderr) = run_python(&out, &[]);
    assert!(!ok);
    assert!(
        stdout.contains("__CRIBO_SOURCEMAP_DIGEST__"),
        "the docstring (module __doc__) must keep the placeholder verbatim: {stdout}"
    );
    // Remapping proves the bootstrap line received the real digest.
    assert!(
        stderr.contains("main.py\", line 4, in <module>"),
        "traceback must remap, proving digest verification passed: {stderr}"
    );
}

#[test]
fn bare_output_filename_anchors_sources_to_current_directory() {
    let dir = fixture_project();
    let output = Command::new(env!("CARGO_BIN_EXE_cribo"))
        .args([
            "--entry",
            "main.py",
            "--output",
            "bundle.py",
            "--sourcemap=linked",
        ])
        .current_dir(dir.path())
        .output()
        .expect("run cribo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "bundling must succeed: {stderr}");

    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map");
    let map = parse_map(&map_json);
    let sources: Vec<&str> = map.get_sources().collect();
    assert!(
        sources.contains(&"helper.py"),
        "a bare output name must resolve sources relative to the current directory: {sources:?}"
    );
    assert!(
        sources.iter().all(|source| !source.starts_with("tmp/")),
        "absolute paths must not be stripped into cwd-relative nonsense: {sources:?}"
    );
}

#[test]
fn runtime_renders_the_traceback_passed_to_excepthook() {
    let dir = make_project(&[
        (
            "main.py",
            "from helper import boom\n\ntry:\n    boom()\nexcept Exception as error:\n    saved = \
             error.__traceback__\n    error.__traceback__ = None\n    \
             __import__(\"sys\").excepthook(type(error), error, saved)\n",
        ),
        (
            "helper.py",
            "def boom():\n    raise ValueError(\"saved traceback kaboom\")\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(
        ok,
        "manual excepthook invocation must not fail the process: {stderr}"
    );
    assert!(
        stderr.contains("helper.py\", line 2, in boom"),
        "the hook argument must drive frame rendering after __traceback__ is cleared: {stderr}"
    );
    assert!(stderr.contains("saved traceback kaboom"), "{stderr}");
}

#[test]
fn wrapper_initialization_call_maps_to_import_site() {
    let dir = make_project(&[
        ("main.py", "import effects\n"),
        (
            "effects.py",
            "print(\"loading effects\")\nraise ValueError(\"wrapper import kaboom\")\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("main.py\", line 1, in <module>"),
        "the synthesized init call must map to `import effects`: {stderr}"
    );
    assert!(
        stderr.contains("effects.py\", line 2"),
        "the wrapper body must remain mapped to effects.py: {stderr}"
    );
    assert!(
        !stderr.contains("bundle.py\", line"),
        "no user-facing frame should expose bundle coordinates: {stderr}"
    );
}

#[test]
fn renamed_function_frames_use_original_names() {
    let dir = make_project(&[
        (
            "main.py",
            "from first import boom as first_boom\nfrom second import boom as second_boom\n\n\
             assert callable(first_boom)\nsecond_boom()\n",
        ),
        ("first.py", "def boom():\n    return \"first\"\n"),
        (
            "second.py",
            "def boom():\n    raise ValueError(\"renamed function kaboom\")\n",
        ),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let bundled_code = fs::read_to_string(&bundle).expect("read bundle");
    assert!(
        bundled_code.contains("def _cribo_"),
        "the fixture must exercise conflict-driven function renaming"
    );
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map");
    let map = parse_map(&map_json);
    assert!(
        map.get_names().any(|name| name == "boom"),
        "the source map must preserve the original function name"
    );

    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("second.py\", line 2, in boom"),
        "traceback names must come from the source map: {stderr}"
    );
    assert!(
        !stderr.contains("in _cribo_"),
        "synthetic conflict-resolution names must not leak: {stderr}"
    );
}

#[test]
fn mapped_frames_keep_pep657_carets() {
    if !python_at_least(11) {
        return;
    }
    let dir = make_project(&[
        ("main.py", "from helper import boom\n\nboom()\n"),
        ("helper.py", "def boom():\n    return 1 + \"boom\"\n"),
    ]);
    let bundle = bundle_crash_project(&dir, "--sourcemap=linked");
    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("helper.py\", line 2, in boom"),
        "the raising frame must remap: {stderr}"
    );
    assert!(
        stderr.contains('^'),
        "mapped frames must retain PEP 657 position indicators: {stderr}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_source_paths_are_encoded_and_decoded() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

    let dir = TempDir::new().expect("create temp dir");
    let mut directory_name = b"src-".to_vec();
    directory_name.push(0xFF);
    let source_dir = dir.path().join(OsString::from_vec(directory_name));
    fs::create_dir_all(&source_dir).expect("create non-UTF-8 source directory");
    fs::write(
        source_dir.join("main.py"),
        "from helper import boom\n\nboom()\n",
    )
    .expect("write main.py");
    fs::write(
        source_dir.join("helper.py"),
        "def boom():\n    raise ValueError(\"non-utf8 path kaboom\")\n",
    )
    .expect("write helper.py");

    let bundle = dir.path().join("bundle.py");
    let output = Command::new(env!("CARGO_BIN_EXE_cribo"))
        .arg("--entry")
        .arg(source_dir.join("main.py"))
        .arg("--output")
        .arg(&bundle)
        .arg("--sourcemap=linked")
        .output()
        .expect("run cribo");
    assert!(
        output.status.success(),
        "bundling must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let map_json = fs::read_to_string(dir.path().join("bundle.py.map")).expect("read map");
    assert!(
        map_json.contains("src-%FF"),
        "invalid path bytes must be encoded losslessly: {map_json}"
    );
    assert!(
        !map_json.contains('\u{FFFD}'),
        "lossy replacement characters must not enter source paths"
    );

    let (ok, _, stderr) = run_python(&bundle, &[]);
    assert!(!ok);
    assert!(
        stderr.contains("helper.py\", line 2, in boom"),
        "the encoded source path must remap: {stderr}"
    );
    assert!(
        stderr.contains("raise ValueError(\"non-utf8 path kaboom\")"),
        "the runtime must reopen the original non-UTF-8 path: {stderr}"
    );
}
