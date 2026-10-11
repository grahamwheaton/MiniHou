"""Route a modifier's template into the one external editor session. No bpy on workers."""
import http.client
import json
import os
from pathlib import Path
import threading
from urllib.parse import urlsplit
import webbrowser


class NetworkEditor:
    def __init__(self, timeout=5.0):
        self.timeout = timeout
        self._lock = threading.Lock()
        self._thread = None
        self._state = {"state": "idle", "path": "", "detail": ""}

    def snapshot(self):
        with self._lock:
            return dict(self._state)

    def _set(self, state, path, detail):
        with self._lock:
            self._state = {"state": state, "path": path, "detail": detail}

    def start(self, server, graph_path):
        if server.get("state") != "ready" or not server.get("editor_url"):
            raise ValueError("Start or reuse a ready MiniHou server first")
        url = urlsplit(server["editor_url"])
        port = url.port if url.port is not None else 80
        if (url.scheme != "http" or url.hostname not in {"127.0.0.1", "localhost", "::1"}
                or not 1 <= port <= 65535
                or url.username or url.password or url.path != "/" or url.query or url.fragment):
            raise ValueError("Server did not provide a valid local editor URL")
        if not graph_path or not Path(graph_path).is_absolute():
            raise ValueError("This modifier has no absolute graph template path; its stored snapshot cannot be edited here")
        path = str(Path(graph_path).resolve())
        with self._lock:
            if self._thread is not None and self._thread.is_alive():
                raise ValueError("Another Edit Network request is in progress; retry when it finishes")
            self._state = {"state": "opening", "path": path, "detail": "Saving current edits and opening graph template..."}
            self._thread = threading.Thread(target=self._open, args=(server["editor_url"], path),
                                            name="minihou-edit-network", daemon=True)
            self._thread.start()

    def _open(self, editor_url, path):
        url = urlsplit(editor_url)
        connection = http.client.HTTPConnection(url.hostname, url.port or 80, timeout=self.timeout)
        selected = False
        try:
            connection.request("GET", "/api/graph")
            current = connection.getresponse()
            description = json.loads(current.read().decode("utf-8"))
            if current.status != 200 or type(description.get("document_id")) is not int:
                raise ValueError("Update/restart the external MiniHou engine for safe Edit Network routing")
            connection.request("POST", "/api/open", json.dumps({"path": path, "create": False}),
                               {"Content-Type": "application/json"})
            response = connection.getresponse()
            data = json.loads(response.read().decode("utf-8"))
            if response.status != 200:
                raise ValueError(data.get("error", f"Editor returned HTTP {response.status}"))
            actual = data.get("path")
            if not actual or os.path.normcase(os.path.realpath(actual)) != os.path.normcase(path):
                raise ValueError("Editor did not confirm the requested graph; browser was not opened")
            selected = True
            if not webbrowser.open(editor_url):
                raise ValueError("Graph template selected, but the OS could not open the browser; open " + editor_url)
            self._set("ready", path, "Template opened. Save edits, then Reload Graph on this modifier.")
        except Exception as exc:
            detail = str(exc)
            if isinstance(exc, OSError) and not selected:
                detail = "Could not confirm editor routing (the session may have switched): " + detail
            self._set("error", path, detail)
        finally:
            connection.close()
