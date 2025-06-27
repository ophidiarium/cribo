from enum import Enum
from typing import TypedDict
from abc import ABC, abstractmethod


class PersonTitle(Enum):
    MR = "Mr."
    MRS = "Mrs."
    MS = "Ms."
    DR = "Dr."
    PROF = "Prof."


class Sex(Enum):
    FEMININE = "Feminine"
    MASCULINE = "Masculine"


class Person:
    def __init__(self, name: str, title: PersonTitle):
        self.name = name
        self.title = title

    def greet(self) -> str:
        return f"{self.title.value} {self.name}"


class Pet(ABC):
    def __init__(self, name: str):
        self.name = name

    @abstractmethod
    def speak(self) -> str:
        pass


def create_mr(name: str) -> Person:
    return Person(name, PersonTitle.MR)


def create_ms(name: str) -> Person:
    return Person(name, PersonTitle.MS)


ALICE_NAME = "Alice"
BOB_NAME = "Bob"


class Phrase(TypedDict):
    what: str
    whom: Person


def say(phrase: Phrase) -> str:
    return f"{phrase['what']} {phrase['whom'].greet()}"


def scream(phrase: Phrase) -> str:
    return say(phrase).upper()
