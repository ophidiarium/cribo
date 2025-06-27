"""Another module with hyphen - demonstrating API client functionality."""


class APIClient:
    def __init__(self, base_url="https://api.example.com"):
        self.base_url = base_url
        self.headers = {"User-Agent": "Hyphenated-Module-Client/1.0"}

    def make_request(self, endpoint):
        return f"Making request to {self.base_url}/{endpoint}"


def create_client(url=None):
    return APIClient(url) if url else APIClient()


API_VERSION = "2.0-stable"
SUPPORTED_ENDPOINTS = ["users", "posts", "comments"]
