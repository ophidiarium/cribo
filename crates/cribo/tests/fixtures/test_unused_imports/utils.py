# Module with side effect imports and complex usage patterns
import logging  # Side effect import - should not be removed
from datetime import datetime, timezone, timedelta  # datetime used, others unused
import subprocess  # Used in conditional
from pathlib import Path  # Used
import threading  # Unused
from typing import *  # Star import - complex case

# Configure logging (side effect)
logging.basicConfig(level=logging.INFO)


def process_file(file_path: str) -> Optional[Dict[str, Any]]:
    """Process a file and return metadata."""
    path = Path(file_path)

    if not path.exists():
        return None

    # Use datetime
    timestamp = datetime.now()

    # Conditional use of subprocess
    if path.suffix == ".py":
        try:
            result = subprocess.run(["python", "-m", "py_compile", str(path)], capture_output=True, text=True)
            compiled = result.returncode == 0
        except:
            compiled = False
    else:
        compiled = None

    return {"path": str(path), "size": path.stat().st_size, "timestamp": timestamp.isoformat(), "compiled": compiled}


class FileProcessor:
    def __init__(self):
        self.logger = logging.getLogger(__name__)

    def process(self, files: List[str]) -> List[Dict[str, Any]]:
        results = []
        for file_path in files:
            self.logger.info(f"Processing {file_path}")
            result = process_file(file_path)
            if result:
                results.append(result)
        return results
