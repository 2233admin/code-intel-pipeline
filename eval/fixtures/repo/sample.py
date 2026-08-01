"""Sample fixture module for eval self-tests.

WHY_RATIONALE: threshold below is 42 because the fixture needs a constant
whose justification only exists as prose in this docstring, far from any
function declaration, to prove artifact-guided search cannot reach
prose-only rationale that a symbol index never captures.
"""

THRESHOLD = 42

# spacer line 11
# spacer line 12
# spacer line 13
# spacer line 14
# spacer line 15
# spacer line 16
# spacer line 17
# spacer line 18
# spacer line 19
# spacer line 20


def add(a, b):
    """Sum two numbers."""
    return a + b


def greet(name):
    """Greet by name."""
    return f"hello {name}"
