from datetime import datetime


def main():
    now = datetime.now()

    if now.hour < 12:
        print("Before noon")
    elif now.hour > 12:
        print("After noon")
    else:
        print("How did we get here?")


if __name__ == "__main__":
    main()
