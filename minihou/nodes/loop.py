"""Repeat boundaries are evaluated as a logical region, never as cyclic graph edges."""
from ..sdk import Input, Int, node

MAX_ITERATIONS = 1000


@node("loop_begin", inputs=["geo"], category="flow")
def loop_begin(geo):
    """Loop Begin: initial geometry on entry, previous result on subsequent iterations.

    Pair with Loop End's begin input. In the body, @iteration is zero-based and
    @numiterations is the End's repeat count. Display the End to evaluate the block.
    """
    raise ValueError("Loop Begin must be evaluated through its paired Loop End")


@node("loop_end", inputs=[Input("geo", doc="Last geometry in the repeated body"),
                          Input("begin", doc="Pairing wire from this block's Loop Begin")],
      category="flow", params=[Int("iterations", 3, min=0, max=MAX_ITERATIONS,
                                   label="Repeat Count", doc="Repeat the body N times; zero returns initial geometry")])
def loop_end(geo, begin, iterations):
    """Loop End: repeat the paired Begin/body region, feeding each result back internally.

    Connect Begin directly to the begin pairing port, and the body's last node to geo.
    Nested blocks are supported; external inputs remain unchanged during the repeat.
    """
    raise ValueError("Loop End requires graph evaluation (it is not a standalone body node)")
