from ..geometry import Geometry
from ..sdk import Input, Int, Str, node


@node("output", inputs=["geo"], category="output", params=[
    Str("object", "", doc="Blender object that shows this output (created and updated live). Empty: the main output, "
                          "shown through the MiniHou proxy; only the display node is the main output"),
])
def output(geo, object=""):
    """Marks the geometry the live link publishes. The evaluator validates it (P present, indices in range). Give an
    output an ``object`` name to publish more than one result: each named output (that is not the display node) becomes
    its own mesh object in Blender."""
    return geo


@node("input", inputs=[Input("preview", optional=True)], category="system", params=[
    Int("index", 0, min=0, max=15, doc="Which input of the system this is (0 = first)"),
    Str("name", "geo", doc="Name of the system's input port"),
])
def input_(preview, index, name):
    """An input of a system (a graph saved in the node folder and used as a node). Inside the system it receives what is
    wired into the system node; when the system graph is opened on its own it passes ``preview`` through, so you can
    build the system on stand-in geometry."""
    return preview if preview is not None else Geometry()
