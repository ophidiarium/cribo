import os
import re
from pathlib import Path


def count_function_lines(file_path):
    """Count lines for all functions in a Rust file."""
    try:
        with open(file_path, "r", encoding="utf-8") as f:
            lines = f.readlines()
    except Exception:
        return []

    functions = []
    i = 0

    while i < len(lines):
        line = lines[i].rstrip()

        # Look for function definitions with proper indentation (not nested in other blocks)
        if re.match(r"^[ ]{0,8}(pub\s+)?fn\s+[a-zA-Z_][a-zA-Z0-9_]*", line):
            func_start = i
            brace_count = 0
            found_opening_brace = False
            func_end = i

            # Count braces to find the function end
            for j in range(i, len(lines)):
                current_line = lines[j]

                # Count opening and closing braces
                for char in current_line:
                    if char == "{":
                        brace_count += 1
                        found_opening_brace = True
                    elif char == "}":
                        brace_count -= 1

                        # If we've closed all braces and found at least one opening brace
                        if brace_count == 0 and found_opening_brace:
                            func_end = j
                            break

                if brace_count == 0 and found_opening_brace:
                    break

            # Extract function name
            func_match = re.search(r"fn\s+([a-zA-Z_][a-zA-Z0-9_]*)", line)
            if func_match:
                func_name = func_match.group(1)
                line_count = func_end - func_start + 1

                # Only include functions with reasonable size (avoid counting errors)
                if line_count > 5 and line_count < 1000:
                    functions.append({"name": func_name, "file": str(file_path), "start_line": func_start + 1, "end_line": func_end + 1, "line_count": line_count})

            i = func_end + 1
        else:
            i += 1

    return functions


def main():
    src_dir = Path("crates/cribo/src")
    all_functions = []

    # Find all .rs files
    for rust_file in src_dir.rglob("*.rs"):
        functions = count_function_lines(rust_file)
        all_functions.extend(functions)

    # Also check test files
    test_dir = Path("crates/cribo/tests")
    if test_dir.exists():
        for rust_file in test_dir.rglob("*.rs"):
            functions = count_function_lines(rust_file)
            all_functions.extend(functions)

    # Sort by line count (descending)
    all_functions.sort(key=lambda x: x["line_count"], reverse=True)

    print("Top 10 largest functions by line count:\n")
    for i, func in enumerate(all_functions[:10], 1):
        rel_path = func["file"].replace("/Volumes/workplace/GitHub/ophidiarium/cribo/", "")
        print(f"{i:2d}. {func['name']} ({func['line_count']} lines)")
        print(f"    File: {rel_path}")
        print(f"    Lines: {func['start_line']}-{func['end_line']}")
        print()


if __name__ == "__main__":
    main()
