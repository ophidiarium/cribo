# Minimal package init to trigger the ordering bug
from .abc import Base
from .console import Console


def main():
    console = Console()
    console.print("Testing")


__all__ = ["Base", "Console", "main"]
