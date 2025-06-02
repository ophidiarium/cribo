# Unused Import Trimmer

The Serpen bundler now includes a powerful unused import trimmer that can analyze Python files and remove unused imports using AST rewriting techniques. This is the first step in our comprehensive AST rewriting implementation strategy.

## Features

- **AST-based analysis**: Uses rustpython-parser for accurate Python syntax analysis
- **Smart import detection**: Distinguishes between used and unused imports
- **Partial import trimming**: Removes only unused items from `from` imports
- **Configurable preservation**: Keep specific imports even if unused
- **Code generation**: Uses rustpython-unparser for clean, formatted output
- **Dry-run mode**: Preview changes without modifying files

## Usage

### Basic Usage

```bash
# Analyze and trim unused imports (overwrites original file)
serpen trim script.py

# Preview changes without modifying the file
serpen trim script.py --dry-run

# Save trimmed output to a different file
serpen trim script.py --output clean_script.py
```

### Advanced Options

```bash
# Preserve future imports (default: true)
serpen trim script.py --preserve-future

# Preserve specific imports by pattern
serpen trim script.py --preserve-patterns "django,pytest"

# Verbose output for debugging
serpen --verbose trim script.py --dry-run
```

## Configuration Options

- `--dry-run`: Preview mode - show what would be trimmed without making changes
- `--output <FILE>`: Output file (if not specified, overwrites input file)
- `--preserve-future`: Preserve `__future__` imports even if unused (default: true)
- `--preserve-patterns <PATTERNS>`: Comma-separated patterns for imports to preserve

## Examples

### Example 1: Basic Unused Import Removal

**Input file (`example.py`):**

```python
import os
import sys
import json
from pathlib import Path

def main():
    print(sys.version)
    p = Path('.')
    print(p)

if __name__ == '__main__':
    main()
```

**Command:**

```bash
serpen trim example.py --dry-run
```

**Output:**

```
Found 2 unused imports in "example.py":
  - os (os)
  - json (json)
```

**After trimming:**

```python
import sys
from pathlib import Path

def main():
    print(sys.version)
    p = Path('.')
    print(p)

if __name__ == '__main__':
    main()
```

### Example 2: Partial Import Trimming

**Input file:**

```python
from typing import List, Dict, Optional, Union

def process_data(items: List[str]) -> Dict[str, int]:
    result = {}
    for item in items:
        result[item] = len(item)
    return result
```

**After trimming:**

```python
from typing import List, Dict

def process_data(items: List[str]) -> Dict[str, int]:
    result = {}
    for item in items:
        result[item] = len(item)
    return result
```

The `Optional` and `Union` imports are removed while `List` and `Dict` are preserved.

### Example 3: Preserve Patterns

**Command:**

```bash
serpen trim test_file.py --preserve-patterns "django,pytest" --dry-run
```

This will preserve any imports containing "django" or "pytest" in their qualified names, even if they appear unused.

## Integration with AST Rewriting Strategy

This unused import trimmer serves as Step 1 of our comprehensive AST rewriting implementation strategy for the Serpen bundler. It demonstrates:

1. **AST parsing** using rustpython-parser
2. **Code analysis** and transformation
3. **AST unparsing** using rustpython-unparser
4. **Configuration-driven behavior**

Future steps will expand this foundation to implement:

- Module dependency analysis and resolution
- Import statement rewriting for bundling
- Code transformation for single-file output
- Integration with the existing bundler architecture

## Technical Details

- **Parser**: Uses `rustpython-parser` for Python AST parsing
- **Analyzer**: Reuses existing `UnusedImportAnalyzer` for import detection
- **Transformer**: Custom AST transformation logic for import removal
- **Unparser**: Uses `rustpython-unparser` for code generation
- **Testing**: Comprehensive test suite with snapshot testing

## Performance

The trimmer is designed for efficient processing:

- Single-pass analysis for import detection
- Minimal AST transformation overhead
- Streaming code generation
- Memory-efficient for large files

## Limitations

Current limitations that will be addressed in future iterations:

- Does not handle dynamic imports (`importlib`)
- Limited support for complex import aliasing scenarios
- Basic heuristics for side-effect import detection

## Contributing

The unused import trimmer is part of the larger AST rewriting implementation strategy. See the [AST Rewriting Implementation Strategy](ast_rewriting_implementation_strategy.md) document for the complete roadmap and contribution guidelines.
