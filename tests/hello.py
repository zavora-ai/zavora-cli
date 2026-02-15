def greet(name: str = "World") -> str:
    """Return a greeting for name, wrapped in a short whimsical story.

    The function narrates a tiny scene where a traveler arrives in a town
    and the townsfolk offer a warm welcome to the named guest.
    """
    story = (
        "Once upon a dawn-lit morning, a traveler knocked on the gates of Sunvale. "
        f"The town's baker, seeing you arrive, wiped flour from their hands and called out, \"Hello, {name}!\" "
        "Children in the square waved little flags, and even the wind seemed to hum a cheerful tune. "
        "You took a deep breath, smiled, and felt at home."
    )
    return story


if __name__ == "__main__":
    print(greet())