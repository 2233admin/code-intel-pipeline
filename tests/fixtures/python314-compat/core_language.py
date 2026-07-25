def factorial(value: int) -> int:
    result = 1
    for item in range(2, value + 1):
        result *= item
    return result


def describe(value: object) -> str:
    match value:
        case (left, right):
            return f"pair:{left}:{right}"
        case _:
            return "other"


print(f"factorial={factorial(5)}")
print("evens=" + ",".join(str(item * item) for item in range(5) if item % 2 == 0))
print(f"match={describe((3, 7))}")
