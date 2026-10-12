"""Actual UI draw test for Edit Network on two modifiers; same invocation as blender_source_ui_test.py."""
from pathlib import Path
import runpy

state = runpy.run_path(str(Path(__file__).with_name("blender_source_ui_test.py")))
scope = state["finish"].__globals__
from minihou_link import network_ui

second = state["modifiers"].add_modifier(state["plane"], str(state["ROOT"] / "examples/modifiers/repeat_transform.json"),
                                         port=state["engine"].port)
second.show_viewport = False  # keep the source UI test's 24 live instances visible
network_ui.router._set("error", state["modifiers"].graph_path(second),
                       "Test-only routing error: template missing; stored modifier snapshot is unchanged.")
drawn = state["result"]["network_modifier_draws"] = []
original_draw = network_ui.draw


def counted(layout, mod):
    original_draw(layout, mod)
    if mod.name not in drawn:
        drawn.append(mod.name)


network_ui.draw = counted
original_finish = state["finish"]


def finish():
    if len(drawn) != 2:
        state["result"]["errors"].append("Edit Network did not draw for both modifiers")
    original_finish()


scope["finish"] = finish
