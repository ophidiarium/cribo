# Test with multiple symlinks creating complex apparent circular dependencies
# Real files: A, B
# Symlinks: C -> A, D -> B, E -> A
# Import chain looks like: main -> A -> B -> C (->A) -> D (->B) -> E (->A)
# This appears very circular but it's really just A <-> B

from real_a import start_chain

print("Testing multi-symlink circular dependency")
result = start_chain()
print(f"Chain result: {result}")

# Should work because all the apparent complexity is just A and B
print("SUCCESS: Multi-symlink circular dependency handled!")
