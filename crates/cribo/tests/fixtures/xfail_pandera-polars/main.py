import pandera.polars as pa
import polars as pl
from pandera.typing import DataFrame
from schema import CitySchema


@pa.check_types(lazy=True)
def keep_state_ca(df: DataFrame[CitySchema]) -> DataFrame[CitySchema]:
    keep_df = df.filter(pl.col("state") == "CA")
    return keep_df


@pa.check_types(lazy=True)
def keep_state_fl(df: pa.PolarsData) -> DataFrame[CitySchema]:
    keep_df = df.filter(pl.col("state") == "FL")
    return keep_df


def main() -> None:
    lf = pl.LazyFrame(
        {
            "state": ["FL", "FL", "FL", "CA", "CA", "CA"],
            "city": [
                "Orlando",
                "Miami",
                "Tampa",
                "San Francisco",
                "Los Angeles",
                "San Diego",
            ],
            "price": [8, 12, 10, 16, 20, 18],
        }
    )
    print("ca:")
    ca_lf = keep_state_ca(lf)
    print(ca_lf.collect())
    print()

    lf2 = pl.LazyFrame(
        {
            "state": ["FL", "FL", "FL", "CA", "CA", "CA"],
            "city": [
                "Orlando",
                "Miami",
                "Tampa",
                "San Francisco",
                "Los Angeles",
                "San Diego",
            ],
            "price": [8, 12, 10, 16, 20, 18],
        }
    )
    print("fl:")
    fl_lf = keep_state_fl(lf2)
    print(fl_lf.collect())
    print()

    print("DONE")


if __name__ == "__main__":
    main()
