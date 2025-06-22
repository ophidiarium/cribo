from speaking import ALICE_NAME, create_ms, say, Person, Sex


def main() -> None:
    # Direct usage of imported functions and constants
    print(say({"what": "Hello", "whom": create_ms(ALICE_NAME)}))

    # Usage of Person class (which will keep its dependencies)
    alice = Person(ALICE_NAME, Sex.FEMALE)
    print(f"Created person: {alice.name}")


if __name__ == "__main__":
    main()
