"""Interactive smoke test. Runs *inside a normal (GUI) Blender*, driven by Blender's own event loop.

Started by ``python tools/launch_blender.py --gui-test`` (which also starts the engine). Unlike the headless tests,
nothing here calls ``Runtime.tick()`` by hand: timers, the depsgraph handler, operators and panel drawing all run the
way they do for a user. Results go to ``--out`` (JSON) and screenshots to ``--shots``; Blender quits when done.
"""
import argparse
import json
import os
import sys
import time
import traceback

import bpy

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
for p in (ROOT, os.path.join(ROOT, "blender_addon")):
    if p not in sys.path:
        sys.path.insert(0, p)

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
ap = argparse.ArgumentParser()
ap.add_argument("--port", type=int, required=True)
ap.add_argument("--graph", required=True, help="the graph file the engine is serving (edited for the hot-reload check)")
ap.add_argument("--out", required=True)
ap.add_argument("--shots", required=True)
ap.add_argument("--keep-open", action="store_true", help="do not quit at the end")
ap.add_argument("--reload-cycles", type=int, default=0, help="extra remove/add reload cycles (stress test)")
ap.add_argument("--no-clicks", action="store_true",
                help="use the no-OS-input sidebar check even where xdotool exists (what Windows/macOS do)")
ap.add_argument("--popover-during-reload", action="store_true",
                help="keep the panel open as a popover during reloads. Reproduces a crash inside Blender 5.1 (popup + "
                     "live reload); users cannot reach this state, the add-on never shows its panel as a popover")
ARGS = ap.parse_args(argv)
os.makedirs(ARGS.shots, exist_ok=True)

import minihou_link  # noqa: E402

try:
    minihou_link.unregister()
except Exception:  # noqa: BLE001
    pass
minihou_link.register()
# This smoke test deliberately exercises the optional legacy sidebar.
addon_preferences = bpy.context.preferences.addons.get("minihou_link")
if addon_preferences is None:
    addon_preferences = bpy.context.preferences.addons.new()
    addon_preferences.module = "minihou_link"
addon_preferences.preferences.show_sidebar = True
# No startup splash: it is shown after --python scripts run, and while it is open Blender does not draw the sidebar.
# (Linux closed it with an X11 Escape key; on Windows it stayed open and the sidebar never drew.) --factory-startup
# never saves preferences, so this does not stick.
bpy.context.preferences.view.show_splash = False
from minihou_link import proxy, runtime  # noqa: E402

RESULTS = {"blender": bpy.app.version_string, "checks": [], "ok": True, "screenshots": [], "panel_draws": 0,
           "panel_errors": []}
T0 = time.time()


def log(msg):
    print(f"[gui-test {time.time() - T0:6.2f}s] {msg}", flush=True)


def check(name, ok, detail=""):
    RESULTS["checks"].append({"name": name, "ok": bool(ok), "detail": str(detail)})
    RESULTS["ok"] = RESULTS["ok"] and bool(ok)
    log(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  ({detail})" if detail else ""))


# Count real panel draws (and any exception they raise) without changing behaviour.
_orig_draw = minihou_link.MINIHOU_PT_main.draw


def _counting_draw(self, context):
    RESULTS["panel_draws"] += 1
    try:
        return _orig_draw(self, context)
    except Exception:  # noqa: BLE001
        RESULTS["panel_errors"].append(traceback.format_exc())
        raise


minihou_link.MINIHOU_PT_main.draw = _counting_draw


def view3d():
    for win in bpy.context.window_manager.windows:
        for area in win.screen.areas:
            if area.type == "VIEW_3D":
                region = next(r for r in area.regions if r.type == "WINDOW")
                return win, area, region
    return None, None, None


def screenshot(name):
    path = os.path.join(ARGS.shots, f"{name}.png")
    if os.environ.get("MINIHOU_X11_SHOT"):
        # Under Xvfb, bpy.ops.screen.screenshot returns black (it reads the GL front buffer, which Xvfb does
        # not keep); capture the X screen instead.
        rc = os.system(f'import -window root "{path}"')
        if rc == 0:
            RESULTS["screenshots"].append(path)
            log(f"screenshot {path} (x11)")
            return
    win, area, _ = view3d()
    try:
        with bpy.context.temp_override(window=win, area=area):
            bpy.ops.screen.screenshot(filepath=path, check_existing=False)
        RESULTS["screenshots"].append(path)
        log(f"screenshot {path}")
    except Exception as e:  # noqa: BLE001
        log(f"screenshot failed: {e!r}")


def show_panel(win, area):
    """Draw the MiniHou panel through Blender's real UI code as a popover. (The sidebar's active tab cannot be
    set from Python -- Region.active_panel_category is read-only -- so the sidebar may show another tab.)"""
    with bpy.context.temp_override(window=win, area=area):
        return bpy.ops.wm.call_panel(name="MINIHOU_PT_main", keep_open=True)


def open_sidebar_tab(win, area):
    """Make the sidebar show the MiniHou tab by clicking down the tab strip (X11 + xdotool only). The click that
    makes our panel draw is the right tab; the panel-draw counter is the feedback. Generator; returns True/False."""
    if not (os.environ.get("MINIHOU_X11_SHOT") and os.system("command -v xdotool >/dev/null 2>&1") == 0):
        return False
    ui = next(r for r in area.regions if r.type == "UI")
    screen_h = win.height
    x = ui.x + ui.width - 10                                  # tab strip hugs the region's right edge
    top = screen_h - (ui.y + ui.height)
    for dy in range(15, ui.height - 10, 18):
        before = RESULTS["panel_draws"]
        os.system(f"xdotool mousemove {x} {top + dy} click 1 >/dev/null 2>&1")
        area.tag_redraw()
        yield 0.25
        if RESULTS["panel_draws"] > before:
            os.system(f"xdotool mousemove {ui.x - 300} {top + 200} >/dev/null 2>&1")   # move off the UI
            yield 0.25
            log(f"sidebar tab found at y={top + dy}")
            return True
    return False


TEST_PANELS = []


def show_in_active_tab(win, area):
    """Without OS-level clicks (Windows, macOS): the active sidebar tab can be read but not set from Python, so register
    a copy of the MiniHou panel *in* the active tab. Blender's real UI then draws the add-on's own draw code. Generator;
    returns True once the panel has been drawn."""
    ui = next(r for r in area.regions if r.type == "UI")
    # The sidebar's categories exist only after it has drawn once; until then this reads "UNSUPPORTED".
    category = ""
    for _ in range(50):
        category = getattr(ui, "active_panel_category", "") or ""
        if category and category != "UNSUPPORTED":
            break
        area.tag_redraw()
        yield 0.1
    if not category or category == "UNSUPPORTED":
        log(f"sidebar category never resolved ({category!r}); using 'Item'")
        category = "Item"
    if category == "MiniHou":
        cls = None                                         # already showing our tab
    else:
        # An independent Panel class running the add-on's draw(). Not a subclass: registering a subclass of a
        # registered panel breaks the original's popover (wm.call_panel stops drawing it), even after unregistering.
        src = minihou_link.MINIHOU_PT_main
        attrs = {"bl_idname": "MINIHOU_PT_gui_test_copy", "bl_label": "MiniHou (test copy)", "bl_category": category,
                 "bl_space_type": src.bl_space_type, "bl_region_type": src.bl_region_type, "bl_order": 0,
                 "draw": src.draw}
        if "poll" in src.__dict__:
            attrs["poll"] = src.__dict__["poll"]
        cls = type("MINIHOU_PT_gui_test_copy", (bpy.types.Panel,), attrs)
        bpy.utils.register_class(cls)
        TEST_PANELS.append(cls)
    for _ in range(40):
        before = RESULTS["panel_draws"]
        area.tag_redraw()
        yield 0.1
        if RESULTS["panel_draws"] > before:
            log(f"sidebar drew the panel in the active tab {category!r}")
            return True
    return False


def wait(pred, timeout, name):
    t = time.time()
    while not pred():
        if time.time() - t > timeout:
            check(name, False, f"timed out after {timeout}s")
            return False
        yield 0.05
    check(name, True, f"{time.time() - t:.2f}s")
    return True


def sleep(seconds):
    end = time.time() + seconds
    while time.time() < end:
        yield 0.05


def evaluated_P():
    import numpy as np
    obj = bpy.data.objects[proxy.PROXY_OBJECT]
    me = obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data
    a = np.empty(len(me.vertices) * 3, np.float32)
    me.vertices.foreach_get("co", a)
    return a.reshape(-1, 3)


def expected_P(values):
    """Cook the graph file in this process with the given exposed values (the engine's numpy reference)."""
    from minihou import Evaluator, Graph
    g = Graph.load(ARGS.graph, check=False)  # match the engine: cook the displayed branch
    for name, v in values.items():
        g.set_exposed(name, list(v) if isinstance(v, tuple) else v)
    return Evaluator().cook(g).P


def scenario():
    import numpy as np
    rt = runtime.get_runtime()
    win, area, region = view3d()
    check("found a 3D viewport in the GUI", area is not None)

    # 1. connect the way a user does: type host/port into the panel, press Connect
    wm = bpy.context.window_manager
    wm.minihou_host, wm.minihou_port = "127.0.0.1", ARGS.port
    with bpy.context.temp_override(window=win, area=area, region=region):
        bpy.ops.minihou.connect()
    if not (yield from wait(lambda: rt.applied_count >= 1, 60, "first geometry arrives (driven only by bpy.app.timers)")):
        return
    check("timer is registered with Blender", bpy.app.timers.is_registered(runtime.timer_entry))
    check("output object is hidden but proxy shows the geometry",
          bpy.data.objects[proxy.OUTPUT_OBJECT].hide_get() and len(evaluated_P()) == rt.counts["point"],
          f"{len(evaluated_P())} evaluated points")

    # 2. UI: select proxy, open the sidebar on the MiniHou tab, frame it
    with bpy.context.temp_override(window=win, area=area, region=region):
        bpy.ops.minihou.select_proxy()
        bpy.ops.view3d.view_selected()
    area.spaces.active.show_region_ui = True
    if os.environ.get("MINIHOU_X11_SHOT") and not os.environ.get("MINIHOU_KEEP_SPLASH_KEY"):
        os.system("xdotool key Escape >/dev/null 2>&1")     # belt and braces: close any splash
        yield 0.3
    if os.environ.get("MINIHOU_X11_SHOT") and not ARGS.no_clicks:
        SIDEBAR = yield from open_sidebar_tab(win, area)
        check("sidebar shows the MiniHou tab (clicked like a user)", SIDEBAR)
    else:
        SIDEBAR = yield from show_in_active_tab(win, area)
        check("sidebar draws the MiniHou panel (in the active tab; no OS clicks)", SIDEBAR)
    screenshot("1_connected")

    # 3. a slider drag: 30 value changes at ~60 Hz. Only the real depsgraph handler forwards them.
    mod = bpy.data.objects[proxy.PROXY_OBJECT].modifiers[proxy.MODIFIER]
    ident = proxy.mapping(mod.node_group)["Hill Height"]
    sent0, applied0 = rt.sent_count, rt.applied_count
    t_drag = time.perf_counter()
    for i in range(30):
        proxy.set_input(mod, ident, (0.0, 0.0, 1.2 + 0.04 * (i + 1)))
        mod.id_data.update_tag()
        yield 1 / 60
    drag_s = time.perf_counter() - t_drag
    final = (0.0, 0.0, float(np.float32(1.2 + 0.04 * 30)))
    yield from wait(lambda: rt.forwarder.known.get("Hill Height", (0, 0, 0))[2] == final[2] and rt.applied_count > applied0,
                    20, "drag reached the engine and came back")
    # Let the last update land: on slow software-GL runners the final cook and apply can take well over a second,
    # so poll for the result (bounded) instead of sleeping a fixed time, then wait for applies to stop.
    known = {k: v for k, v in rt.forwarder.known.items()}
    deadline = time.time() + 20
    while time.time() < deadline:
        got, want = evaluated_P(), expected_P(known)
        if got.shape == want.shape and np.allclose(got, want, atol=1e-4):
            break
        yield 0.1
    quiet_since, last = time.time(), rt.applied_count
    while time.time() - quiet_since < 1.0 and time.time() < deadline + 10:
        if rt.applied_count != last:
            quiet_since, last = time.time(), rt.applied_count
        yield 0.05
    sent = rt.sent_count - sent0
    # Timer granularity (~15.6 ms on Windows) and slow software redraws stretch the "60 Hz" drag, so the
    # bound is the forwarding rate limit over the drag's real duration, not a fixed count.
    limit = int(drag_s * 30) + 2
    check("drag was forwarded by the depsgraph handler and rate limited", 1 <= sent <= min(30, limit),
          f"{sent} set_param for 30 edits over {drag_s:.2f}s (limit {limit})")
    known = {k: v for k, v in rt.forwarder.known.items()}
    got, want = evaluated_P(), expected_P(known)
    check("evaluated proxy mesh matches the engine after the drag", got.shape == want.shape and np.allclose(got, want, atol=1e-4),
          f"{got.shape} vs {want.shape}, max err {float(np.abs(got - want).max()) if got.shape == want.shape else 'n/a'}")

    # 4. no feedback loop while the UI keeps redrawing
    s1, a1 = rt.sent_count, rt.applied_count
    for _ in range(60):
        area.tag_redraw()
        yield 0.05
    check("idle with redraws: nothing forwarded, nothing re-applied", rt.sent_count == s1 and rt.applied_count == a1,
          f"sent +{rt.sent_count - s1}, applied +{rt.applied_count - a1}")
    check("no errors reported by the runtime", not rt.last_error, rt.last_error)

    # Blender's own modifier panel in the Properties editor shows the same inputs: keep it on screen for the reloads.
    props_area = next((a for a in win.screen.areas if a.type == "PROPERTIES"), None)
    if props_area is not None:
        try:
            props_area.spaces.active.context = "MODIFIER"
            props_area.tag_redraw()
        except Exception as e:  # noqa: BLE001
            log(f"could not show the modifier tab: {e!r}")
    yield 0.5

    # 5. hot reload while the panel (sidebar, if we could open it) shows the modifier inputs: add an exposed
    #    parameter and change a value in the JSON file. This used to crash Blender (UI kept pointers to the
    #    modifier properties that a full interface rebuild freed).
    def edit_graph(fn):
        with open(ARGS.graph, encoding="utf-8") as f:
            data = json.load(f)
        fn(data)
        time.sleep(0.05)
        with open(ARGS.graph, "w", encoding="utf-8", newline="\n") as f:
            json.dump(data, f, indent=2)

    edit_graph(lambda d: (d["exposed"].append({"name": "Spike Size", "node": "spike", "param": "size_x"}),
                          d["nodes"]["hills"]["params"].__setitem__("frequency", 0.5)))
    yield from wait(lambda: "Spike Size" in proxy.mapping(mod.node_group), 20, "hot reload rebuilt the modifier inputs")
    m = proxy.mapping(mod.node_group)
    check("engine value pushed into the modifier", abs(proxy.get_input(mod, m["Hill Frequency"]) - 0.5) < 1e-6, proxy.get_input(mod, m["Hill Frequency"]))
    check("unchanged inputs kept their identifiers (no full rebuild)", m["Hill Height"] == ident, f"{ident} -> {m['Hill Height']}")
    s2 = rt.sent_count
    d0 = RESULTS["panel_draws"]
    for _ in range(30):
        area.tag_redraw()
        yield 0.05
    check("no echo after the engine pushed new values", rt.sent_count == s2, f"sent +{rt.sent_count - s2}")
    if SIDEBAR:
        check("sidebar panel kept drawing after the reload, without errors",
              RESULTS["panel_draws"] > d0 and not RESULTS["panel_errors"], f"{RESULTS['panel_draws'] - d0} draws")
    screenshot("2_after_reload")

    # 6. a second reload, removing and adding inputs
    if ARGS.popover_during_reload:
        show_panel(win, area)
        yield from sleep(0.5)
    edit_graph(lambda d: d.__setitem__("exposed", [e for e in d["exposed"] if e["name"] not in ("Seed", "Spike Size")]
                                       + [{"name": "Count", "node": "points", "param": "count"}]))
    yield from wait(lambda: "Count" in proxy.mapping(mod.node_group) and "Seed" not in proxy.mapping(mod.node_group),
                    20, "second reload removed/added inputs")
    d1 = RESULTS["panel_draws"]
    for _ in range(20):
        area.tag_redraw()
        yield 0.05
    check("Blender survived the reload; panel drew without errors",
          (RESULTS["panel_draws"] > d1 or not (SIDEBAR or ARGS.popover_during_reload)) and not RESULTS["panel_errors"],
          f"{RESULTS['panel_draws'] - d1} draws")
    names = [i.name for i in mod.node_group.interface.items_tree if i.item_type == "SOCKET" and i.in_out == "INPUT"]
    check("node group inputs follow the graph order", names == ["Geometry", "Hill Height", "Hill Frequency",
                                                                "Scatter Count", "Grid Columns", "Count"], names)
    screenshot("3_after_second_reload")
    # 7. optional stress: repeatedly remove and re-add inputs while the UI shows them
    if ARGS.reload_cycles:
        log(f"stress: {ARGS.reload_cycles} reload cycles")
        for c in range(ARGS.reload_cycles):
            drop = c % 2 == 0
            edit_graph(lambda d: d.__setitem__("exposed", [e for e in d["exposed"] if e["name"] != "Grid Columns"]
                                               if drop else d["exposed"] + [{"name": "Grid Columns", "node": "ground",
                                                                           "param": "columns"}]))
            ok = yield from wait(lambda: ("Grid Columns" in proxy.mapping(mod.node_group)) != drop, 20, f"stress reload {c + 1}")
            for _ in range(6):
                area.tag_redraw()
                yield 0.05
            if not ok:
                break
        check("stress reloads finished without errors", not RESULTS["panel_errors"] and not rt.last_error, rt.last_error)

    # 8. panel drawn by Blender's own UI code, as a popover. Done last on purpose: a keep-open popover that is still open
    #    while a reload removes inputs crashes Blender 5.1 (README, "Known Blender behaviour"), and on Windows the test
    #    cannot close it again.
    if not ARGS.popover_during_reload:
        for cls in TEST_PANELS:                           # only the add-on's own panel from here on
            bpy.utils.unregister_class(cls)
        TEST_PANELS.clear()
        yield 0.2
        d0 = RESULTS["panel_draws"]
        if os.environ.get("MINIHOU_X11_SHOT"):            # popovers open at the pointer: make sure Blender has seen one
            os.system(f"xdotool mousemove {area.x + area.width // 3} {win.height - (area.y + area.height // 2)} >/dev/null 2>&1")
            yield 0.3
        log(f"call_panel: {show_panel(win, area)}")
        yield from sleep(1.5)
        check("Blender's UI drew the MiniHou panel (popover)", RESULTS["panel_draws"] > d0,
              f"{RESULTS['panel_draws'] - d0} draws")
        check("panel draw raised no exceptions", not RESULTS["panel_errors"],
              RESULTS["panel_errors"][-1][-300:] if RESULTS["panel_errors"] else "")
        screenshot("4_panel_popover")

    RESULTS["stats"] = {"applied": rt.applied_count, "sent": rt.sent_count, "counts": rt.counts,
                        "last_timings": {k: v for k, v in rt.last_timings.items() if k != "warnings"},
                        "latency_ms": rt.last_latency_ms, "cook_ms": rt.last_cook_ms}


_gen = scenario()


def _finish():
    RESULTS["seconds"] = round(time.time() - T0, 2)
    with open(ARGS.out, "w", encoding="utf-8") as f:
        json.dump(RESULTS, f, indent=2)
    log(f"done: {'ALL PASSED' if RESULTS['ok'] else 'FAILURES'} -> {ARGS.out}")
    if not ARGS.keep_open:
        runtime.get_runtime().disconnect()
        win = bpy.context.window_manager.windows[0]
        with bpy.context.temp_override(window=win):
            bpy.ops.wm.quit_blender()


def _step():
    try:
        return next(_gen)
    except StopIteration:
        _finish()
    except Exception:  # noqa: BLE001
        check("scenario raised", False, traceback.format_exc()[-800:])
        _finish()
    return None


bpy.app.timers.register(_step, first_interval=2.0)
log(f"scheduled; Blender {bpy.app.version_string}, engine port {ARGS.port}")
