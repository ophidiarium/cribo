# Test case where symlinks create an apparent circular dependency
# when judged by module names, but not by actual files

# Import chain that appears circular:
# main -> moduleA -> moduleB -> moduleC (which is a symlink to moduleA)
# This looks like: A -> B -> C -> A (circular!)
# But actually is: A -> B -> A (just A importing B, B importing A)

from moduleA import funcA, get_chain

print("Testing symlink circular dependency handling")
print(f"funcA result: {funcA()}")
print(f"Chain: {get_chain()}")

# The chain should work because moduleC is just moduleA
# So there's no real circular dependency, just A <-> B
print("SUCCESS: Symlink circular dependency handled correctly!")
