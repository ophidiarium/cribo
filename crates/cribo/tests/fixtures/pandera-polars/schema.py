import pandera.polars as pa


class CitySchema(pa.DataFrameModel):
    state: str
    city: str
    price: int
