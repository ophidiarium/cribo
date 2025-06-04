#!/usr/bin/env python3
"""
Class name collision test.
Both modules define classes with the same names.
"""

from models import User as ModelUser, Product as ModelProduct
from entities import User as EntityUser, Product as EntityProduct


def main():
    # Create instances from both modules
    model_user = ModelUser("Alice", "alice@example.com")
    entity_user = EntityUser("Bob", 25)

    model_product = ModelProduct("Widget", 19.99)
    entity_product = EntityProduct("P001", "Gadget")

    print(f"Model User: {model_user}")
    print(f"Entity User: {entity_user}")
    print(f"Model Product: {model_product}")
    print(f"Entity Product: {entity_product}")

    return {"model_user": str(model_user), "entity_user": str(entity_user), "model_product": str(model_product), "entity_product": str(entity_product)}


if __name__ == "__main__":
    result = main()
    print("Result:", result)
