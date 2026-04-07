print("loading loops")  # side effect -> wrapper module

last_result = "none"
loop_count = 0


def find_item(items, target):
    global last_result
    for item in items:
        if item == target:
            last_result = f"found:{target}"
            break
    else:
        last_result = f"missing:{target}"
    return last_result


def count_up(limit):
    global loop_count
    loop_count = 0
    while loop_count < 5:
        loop_count += 1
        if loop_count >= limit:
            break
    else:
        loop_count = -1  # sentinel: loop completed without break
    return loop_count
