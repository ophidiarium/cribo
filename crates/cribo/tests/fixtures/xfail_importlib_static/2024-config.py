"""Module starting with a number - can't be imported with regular import."""


class Config2024:
    YEAR = 2024
    VERSION = "2024.1.0"
    FEATURES = ["new-ui", "dark-mode", "api-v3"]

    @classmethod
    def get_config(cls):
        return {"year": cls.YEAR, "version": cls.VERSION, "features": cls.FEATURES}


def load_yearly_config():
    return Config2024()


SUPPORTED_YEARS = [2022, 2023, 2024]
CONFIG_PREFIX = "config-"
