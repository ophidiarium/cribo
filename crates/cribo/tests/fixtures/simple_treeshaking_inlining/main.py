from speaking import ALICE_NAME, create_ms, say


def main() -> None:
    print(say({"what": "Hello", "whom": create_ms(ALICE_NAME)}))


if __name__ == "__main__":
    main()
    # print(
    #     "Total globals defined:",
    #     {s for s in globals() if isinstance(s, str) and not s.startswith("__")},
    # )
