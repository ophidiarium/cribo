class Empty:
    pass


class Empty2:
    pass


class Inheriting(Empty):
    pass


class Meta(Empty2, Inheriting, metaclass=type):
    pass
