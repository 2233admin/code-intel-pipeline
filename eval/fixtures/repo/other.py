"""Other fixture module: exists to test cross-file keyword collisions."""


def add(x, y):
    # A second, unrelated 'add' -- same name as the one in the sibling
    # fixture module, to prove Arm A's AND-keyword matching can
    # disambiguate by pairing a name with a path-fragment keyword.
    return x - y
