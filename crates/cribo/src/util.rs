use std::path::Path;

use cow_utils::CowUtils;

/// Convert a relative path to a Python module name, handling .py extension and __init__.py
pub fn module_name_from_relative(relative_path: &Path) -> Option<String> {
    let mut parts: Vec<String> = relative_path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();

    if parts.is_empty() {
        return None;
    }

    let last_part = parts.last_mut()?;
    // Remove .py extension
    if Path::new(last_part)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("py"))
    {
        *last_part = last_part[..last_part.len() - 3].to_owned();
    }

    // Handle __init__.py and __main__.py files
    if last_part == "__init__" || last_part == "__main__" {
        parts.pop();
    }

    // Skip files that don't map to a module
    if parts.is_empty() {
        return None;
    }

    Some(parts.join("."))
}

/// Normalize line endings to LF (\n) for cross-platform consistency
/// This ensures reproducible builds regardless of the platform where bundling occurs
pub fn normalize_line_endings(content: &str) -> String {
    // Replace Windows CRLF (\r\n) and Mac CR (\r) with Unix LF (\n)
    content
        .cow_replace("\r\n", "\n")
        .cow_replace('\r', "\n")
        .into_owned()
}

#[cfg(test)]
mod tests {}
