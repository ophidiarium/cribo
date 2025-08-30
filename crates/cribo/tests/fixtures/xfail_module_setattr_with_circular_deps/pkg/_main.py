"""Main module that imports client which imports back to parent."""

from ._client import Client


def main():
    """Main function using Client."""
    client = Client()
    return client.make_request()
