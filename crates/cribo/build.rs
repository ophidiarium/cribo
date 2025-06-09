use std::{env, fs, io, path::PathBuf};

fn main() -> io::Result<()> {
    // During test builds, override the default temporary directory to a workspace-local path
    // This is particularly useful for OpenAI codex which runs tests in a sandboxed environment
    if env::var("CARGO_CFG_TEST").is_ok() {
        if let Ok(manifest_dir) = env::var("CARGO_MANIFEST_DIR") {
            let target_tmp = PathBuf::from(manifest_dir).join("target").join("tmp");
            fs::create_dir_all(&target_tmp)?;
            // Instruct Rust test harness to use this directory for temp files
            println!("cargo:rustc-env=TMPDIR={}", target_tmp.display());

            if env::var("VIRTUAL_ENV").is_err() {
                let cwd = env::current_dir()?;
                let candidate = cwd.join(".venv");
                if candidate.is_dir() {
                    let abs = candidate.canonicalize()?;
                    println!("cargo:rustc-env=VIRTUAL_ENV={}", abs.display());
                }
            }
        }
    }

    Ok(())
}
