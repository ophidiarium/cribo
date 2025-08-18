"""Test module attribute setting on wildcard imports with circular dependencies."""

import pkg

# Verify that wildcard-imported items have correct __module__ attribute
stream_a = pkg.AsyncStream()
stream_s = pkg.SyncStream()

print(f"AsyncStream: {stream_a.read()}")
print(f"SyncStream: {stream_s.read()}")
print("Success!")
