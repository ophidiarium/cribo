from __future__ import annotations

from typing import Dict, List, Any


def process_data(data: Dict[str, Any]) -> Dict[str, Any]:
    """Process input data and return results.

    This function uses forward references in type hints.
    """
    result: ProcessingResult = {"input": data, "processed": True, "output": _transform_data(data)}
    return result


def _transform_data(data: Dict[str, Any]) -> List[str]:
    """Transform data into list format."""
    return [f"{k}={v}" for k, v in data.items()]


# Forward reference that requires future import
ProcessingResult = Dict[str, Any]
