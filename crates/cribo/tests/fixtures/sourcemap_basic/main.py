from calculator import add, multiply
from utils import describe

total = add(2, 3)
product = multiply(total, 4)
print(describe("total", total))
print(describe("product", product))
