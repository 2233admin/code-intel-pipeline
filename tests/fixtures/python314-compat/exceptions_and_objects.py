class Counter:
    def __init__(self) -> None:
        self.value = 0

    def add(self, amount: int) -> None:
        self.value += amount


counter = Counter()
counter.add(5)
print(f"counter={counter.value}")

try:
    1 / 0
except ZeroDivisionError as error:
    print(f"error={type(error).__name__}")
finally:
    print("finally=done")
