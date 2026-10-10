from ..sdk import node


@node("output", inputs=["geo"], category="output")
def output(geo):
    """Marks the geometry the live link publishes. The evaluator validates it (P present, indices in range)."""
    return geo
