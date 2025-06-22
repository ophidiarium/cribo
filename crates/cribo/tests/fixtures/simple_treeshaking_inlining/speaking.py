"""Module with many symbols, only some of which are used"""

from abc import ABC
from enum import Enum
from typing import Protocol


# This will be tree-shaken
class Pet(ABC):
    pass


# This will be kept (used by create_ms)
class Sex(Enum):
    MALE = "male"
    FEMALE = "female"


# This will be kept (used directly)
ALICE_NAME = "Alice"

# This will be tree-shaken
BOB_NAME = "Bob"


# This will be kept (used directly)
def say(phrase):
    return f"{phrase['what']}, {phrase['whom']}!"


# This will be tree-shaken
def scream(phrase):
    return say(phrase).upper()


# This will be kept (used directly)
def create_ms(name):
    return f"Ms. {name}"


# This will be tree-shaken
def create_mr(name):
    return f"Mr. {name}"


# These will be kept (Person is used, PersonTitle and Phrase are dependencies)
class PersonTitle(Protocol):
    def __str__(self) -> str: ...


class Phrase(Protocol):
    what: str
    whom: str


class Person:
    def __init__(self, name: str, sex: Sex):
        self.name = name
        self.sex = sex

    def title(self) -> PersonTitle:
        if self.sex == Sex.FEMALE:
            return create_ms(self.name)
        else:
            return create_mr(self.name)
