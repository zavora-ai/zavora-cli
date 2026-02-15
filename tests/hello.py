def greet(name: str = "World") -> str:
    """Return a greeting for name."""
    return f"Hello, {name}!"


if __name__ == "__main__":
    print(greet())
