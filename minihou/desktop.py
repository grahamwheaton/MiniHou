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


def run(server, workspace):
    import webview
    if os.name == 'nt':
        import ctypes
        ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID('MiniHou.Desktop')
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
    try:
        webview.start(gui='edgechromium' if os.name == 'nt' else None,
                      icon=str(Path(__file__).parent / 'assets' / 'minihou.ico'))
    finally:
        server.desktop_activate = None
