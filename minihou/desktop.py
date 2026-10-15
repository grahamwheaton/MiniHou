"""Native editor window. Engine discovery also lets subsequent launches raise it."""
import json
import os
import threading
import time
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.parse import urlsplit


def reuse(workspace):
    from .link.discovery import directory
    for record in sorted(directory().glob("*.json"), reverse=True)[:32]:
        try:
            url = json.loads(record.read_text(encoding='utf-8'))['editor_url']
            parsed = urlsplit(url)
            if parsed.scheme != 'http' or parsed.hostname not in {'localhost', '127.0.0.1', '::1'} or parsed.username or parsed.password or parsed.path != '/':
                continue
            request = Request(url + 'api/desktop/activate',
                              json.dumps({'workspace': str(Path(workspace).resolve())}).encode(),
                              {'Content-Type': 'application/json'})
            with urlopen(request, timeout=0.25) as response:
                if json.load(response).get('desktop'):
                    return True
        except (OSError, ValueError, KeyError):
            continue
    return False


def file_types(exts):
    """pywebview's file filter strings for the extensions asked for."""
    exts = [e for e in exts or [] if e]
    return ((f"Supported files ({';'.join('*' + e for e in exts)})", "All files (*.*)") if exts else ("All files (*.*)",))


def make_picker(window, webview):
    """The native open / save dialog of the desktop window, as the engine's ``pick_file`` hook: ``fn(mode, exts, title, start)`` ->
    ``{"path": ...}`` (empty when cancelled). ``start`` is a folder, or a file whose folder and name are offered."""
    def pick(mode, exts, title, start):
        folder, name = (start, "") if start and os.path.isdir(start) else (os.path.dirname(start or "") or "", os.path.basename(start or ""))
        kinds = getattr(webview, "FileDialog", None)
        if mode == "save":
            kind = kinds.SAVE if kinds else webview.SAVE_DIALOG
            result = window.create_file_dialog(kind, directory=folder if os.path.isdir(folder) else "", save_filename=name, file_types=file_types(exts))
        else:
            kind = kinds.OPEN if kinds else webview.OPEN_DIALOG
            result = window.create_file_dialog(kind, directory=folder if os.path.isdir(folder) else "", allow_multiple=False, file_types=file_types(exts))
        if isinstance(result, (list, tuple)):
            result = result[0] if result else ""
        return {"path": str(result or "")}
    return pick


def drop_handler(window):
    """Files dropped on the editor window arrive here with their real paths (a page only ever sees names); they go to the page's
    ``minihou.droppedPaths``, which makes the nodes. Without this the page uploads the file instead."""
    def on_drop(event):
        files = ((event or {}).get("dataTransfer") or {}).get("files") or []
        found = [f.get("pywebviewFullPath") for f in files if isinstance(f, dict) and f.get("pywebviewFullPath")]
        if found:
            x, y = (event or {}).get("clientX"), (event or {}).get("clientY")
            window.evaluate_js(f"window.minihou && window.minihou.droppedPaths({json.dumps(found)}, {json.dumps(x)}, {json.dumps(y)})")
    return on_drop


GPU_FLAG = "--force_high_performance_gpu"


def browser_arguments(environ=os.environ):
    """The WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS value with the fast GPU asked for.

    On a laptop with an integrated and a discrete GPU, WebView2 draws the viewport on the integrated one (measured on an
    i7-14700HX + RTX 4060: 4x slower frames at 4M quads). Arguments already set are kept; ``MINIHOU_GPU=default`` leaves
    the system's choice alone."""
    existing = environ.get("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "")
    if environ.get("MINIHOU_GPU", "").strip().lower() in ("default", "system", "low") or GPU_FLAG in existing:
        return existing
    return (existing + " " + GPU_FLAG).strip()


def run(server, workspace):
    import webview
    if os.name == 'nt':
        import ctypes
        ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID('MiniHou.Desktop')
        os.environ["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = browser_arguments()   # read when the WebView2 environment is made
    window = webview.create_window('MiniHou', server.url, width=1440, height=940,
                                   min_size=(850, 560), background_color='#24282d',
                                   confirm_close=True)

    def activate(body):
        requested = body.get('workspace')
        if requested and os.path.normcase(str(Path(requested).resolve())) != os.path.normcase(str(Path(workspace).resolve())):
            return False
        window.restore()
        window.show()
        return True

    close_state = {"checking": False, "approved": False}

    def finish_close():
        try:
            for _ in range(40):
                if window.evaluate_js("!window.minihou || window.minihou.desktopCanClose()"):
                    break
                time.sleep(0.05)
            else:
                return
            if not server.session.flush():
                window.create_confirmation_dialog('MiniHou - save failed',
                    'Could not save your graph. Keep MiniHou open and check the workspace folder.')
                return
            close_state["approved"] = True
            window.destroy()
        finally:
            close_state["checking"] = False

    def closing():
        if close_state["approved"]:
            close_state["approved"] = False
            return True
        if not close_state["checking"]:
            close_state["checking"] = True
            threading.Thread(target=finish_close, daemon=True).start()
        return False

    window.events.closing += closing
    server.desktop_activate = activate
    server.pick_file = make_picker(window, webview)

    def bind(win):                                    # after the window exists: let dropped files arrive with their real paths
        try:
            from webview.dom import DOMEventHandler
            win.dom.document.events.drop += DOMEventHandler(drop_handler(win), True, True)
        except Exception:  # noqa: BLE001 - an older pywebview: the page uploads dropped files instead
            pass

    try:
        webview.start(bind, window, gui='edgechromium' if os.name == 'nt' else None,
                      icon=str(Path(__file__).parent / 'assets' / 'minihou.ico'))
    finally:
        server.desktop_activate = None
        server.pick_file = None
