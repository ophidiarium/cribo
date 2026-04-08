from loops import find_item, count_up

found = find_item([1, 2, 3], 2)
print(f"found: {found}")

not_found = find_item([1, 2, 3], 99)
print(f"not found: {not_found}")

count = count_up(3)
print(f"count up to 3: {count}")

count2 = count_up(10)
print(f"count up to 10: {count2}")
