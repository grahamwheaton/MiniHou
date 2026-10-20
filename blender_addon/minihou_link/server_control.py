"""External engine lifecycle, independent of bpy. Never installs packages in Blender."""
from __future__ import annotations

import atexit
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import webbrowser
from urllib.parse import urlsplit

from .protocol import DEFAULT_PORT, PROTOCOL_VERSION, recv_message, send_message


class ServerController:
    """One asynchronous launcher; only its own Popen handle can ever be stopped."""

    def __init__(self, *, startup_timeout=30.0):
        self.startup_timeout = startup_timeout
        self._lock = threading.RLock()
        self._state = "stopped"
        self._detail = ""
        self._editor_url = ""
        self._process = None
        self._log = None
        self._thread = None
        self._stop_thread = None
        self._monitor = None
        atexit.register(self._shutdown)
        self._cancel = threading.Event()

    def snapshot(self):
        with self._lock:
            if self._state == "ready" and self._process is not None and self._process.poll() is not None:
                self._state = "error"
                self._detail = "Engine exited: " + self._read_log()
                self._editor_url = ""
            return {"state": self._state, "detail": self._detail,
                    "editor_url": self._editor_url, "owned": self._process is not None}

    def _set(self, state, detail="", editor_url=""):
        with self._lock:
            self._state, self._detail, self._editor_url = state, detail, editor_url

    def start(self, app_path, graph_path, *, host="127.0.0.1",
              port=DEFAULT_PORT, http_port=8766):
        """Return immediately. Poll snapshot() from a Blender timer for status."""
        with self._lock:
            if self._state == "ready" and self._process is None:
                raise ValueError("Disconnect the reused server before changing its configuration")
            if self._state in {"starting", "stopping"}:
                raise ValueError("Server operation already in progress")
            if self._process is not None and self._process.poll() is None:
                raise ValueError("Stop the owned engine before changing its configuration")
            if self._process is not None:
                self._terminate_owned()
            if host not in {"localhost", "127.0.0.1", "::1"}:
                raise ValueError("Start Server supports loopback hosts only")
            if not 1 <= int(port) <= 65535 or not 1 <= int(http_port) <= 65535 or int(port) == int(http_port):
                raise ValueError("Choose distinct link and editor ports from 1 to 65535")
            self._cancel = threading.Event()
            self._set("starting", "Checking for a compatible engine")
            self._thread = threading.Thread(target=self._start, args=(
                app_path, graph_path, host, int(port), int(http_port)),
                name="minihou-launcher", daemon=True)
            self._thread.start()

    @staticmethod
    def _probe(host, port):
        try:
            sock = socket.create_connection((host, port), timeout=2.0)
        except (ConnectionRefusedError, socket.timeout):
            return None
        with sock:
            sock.settimeout(0.7)
            send_message(sock, {"type": "hello", "protocol": PROTOCOL_VERSION,
                                "client": "minihou-launcher", "mode": "instances"})
            header, _ = recv_message(sock)
            if (header.get("type") != "welcome" or header.get("server") != "minihou"
                    or header.get("protocol") != PROTOCOL_VERSION
                    or "instances_v1" not in header.get("capabilities", [])):
                raise ValueError("Port is occupied by an incompatible server")
            return header

    @staticmethod
    def _compatible(header, graph, host, http_port):
        actual = header.get("graph_path")
        if not actual:
            return False
        if os.path.normcase(os.path.realpath(actual)) != os.path.normcase(os.path.realpath(graph)):
            raise ValueError("Existing engine serves a different graph; use its graph or another port")
        if header.get("editor") == "native" and header.get("editor_url") is None:
            return True
        url = urlsplit(header.get("editor_url") or "")
        loopback = {"localhost", "127.0.0.1", "::1"}
        if (url.scheme != "http" or host not in loopback or url.hostname not in loopback
                or url.port != http_port or url.path != "/" or url.query or url.fragment
                or url.username is not None or url.password is not None):
            raise ValueError("Existing engine editor address does not match the configured editor port")
        return True

    @staticmethod
    def _app_executable(app_path):
        name = "minihou-app.exe" if os.name == "nt" else "minihou-app"
        if app_path:
            candidates = [Path(app_path)]
        else:
            root = Path(__file__).resolve().parents[2]
            target = os.environ.get("CARGO_TARGET_DIR")
            targets = [root / "rust" / "target"] if not target else [Path(target)]
            if target and not targets[0].is_absolute():
                targets = [Path.cwd() / target, root / target, root / "rust" / target]
            candidates = [base / profile / name for base in targets for profile in ("debug", "release")]
        for candidate in candidates:
            if candidate.is_absolute() and candidate.is_file():
                return candidate.resolve()
        raise ValueError("MiniHou Rust app (minihou-app) not found. Build it with cargo build --manifest-path rust/Cargo.toml -p minihou-app, or set its executable path in add-on preferences.")

    def _start(self, app_path, graph_path, host, port, http_port):
        try:
            if not graph_path or not Path(graph_path).is_absolute() or not Path(graph_path).is_file():
                raise ValueError("Select an existing absolute graph file path")
            graph = Path(graph_path).resolve()
            existing = self._probe(host, port)
            if existing is not None:
                if not self._compatible(existing, graph, host, http_port):
                    raise ValueError("Existing engine has not reported its graph; retry when it is ready")
                if not self._cancel.is_set():
                    self._set("ready", "Reusing existing engine (not owned by Blender)", existing.get("editor_url") or "")
                    self._monitor_reused(host, port)
                return
            executable = self._app_executable(app_path)
            if host not in {"localhost", "127.0.0.1"} or port != DEFAULT_PORT:
                raise ValueError(f"minihou-app listens on 127.0.0.1:{DEFAULT_PORT}; select that link port to launch it")
            flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
            if self._cancel.is_set():
                return
            if self._log is not None:
                self._log.close()
            self._log = tempfile.TemporaryFile(mode="w+b")
            self._process = subprocess.Popen([str(executable), str(graph)],
                cwd=str(graph.parent), stdin=subprocess.DEVNULL,
                stdout=self._log, stderr=subprocess.STDOUT, creationflags=flags)
            deadline = time.monotonic() + self.startup_timeout
            while not self._cancel.wait(0.15):
                if self._process.poll() is not None:
                    raise RuntimeError("Engine failed to start: " + self._read_log())
                try:
                    header = self._probe(host, port)
                except (ConnectionError, socket.timeout):
                    header = None
                if header and self._compatible(header, graph, host, http_port):
                    self._set("ready", "Rust app ready (started by Blender)", header.get("editor_url") or "")
                    return
                if time.monotonic() >= deadline:
                    raise TimeoutError("Engine readiness timed out: " + self._read_log())
        except Exception as exc:
            self._terminate_owned()
            if not self._cancel.is_set():
                self._set("error", str(exc))

    def _read_log(self):
        if self._log is None:
            return "No process log"
        self._log.flush()
        self._log.seek(0, os.SEEK_END)
        size = self._log.tell()
        self._log.seek(max(0, size - 4000))
        return self._log.read().decode("utf-8", errors="replace").strip()

    def _terminate_owned(self):
        process = self._process
        if process is not None:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
            self._process = None
        if self._log is not None:
            self._log.close()
            self._log = None

    def stop(self, *, wait=False):
        """Never sends termination to a remote server or a PID discovered by probing."""
        if self._stop_thread is not None and self._stop_thread.is_alive():
            if wait:
                self._stop_thread.join()
            return
        self._cancel.set()
        self._set("stopping", "Stopping owned engine")
        startup = self._thread
        def finish():
            if startup is not None and startup is not threading.current_thread():
                startup.join()
            if self._monitor is not None:
                self._monitor.join()
            self._terminate_owned()
            self._set("stopped", "Owned engine stopped; external engines were left running")
        thread = threading.Thread(target=finish, name="minihou-stop", daemon=True)
        self._stop_thread = thread
        thread.start()
        if wait:
            thread.join()

    def _monitor_reused(self, host, port):
        def monitor():
            while not self._cancel.wait(2.0):
                try:
                    if self._probe(host, port) is None:
                        raise ConnectionError("External engine is no longer reachable")
                except Exception as exc:
                    if not self._cancel.is_set():
                        self._set("error", str(exc))
                    return
        self._monitor = threading.Thread(target=monitor, name="minihou-health", daemon=True)
        self._monitor.start()

    def _shutdown(self):
        """Interpreter exit cannot create a new thread; finish only owned work."""
        self._cancel.set()
        if self._stop_thread is not None and self._stop_thread.is_alive():
            self._stop_thread.join()
        elif self._thread is not None and self._thread.is_alive():
            self._thread.join()
        if self._monitor is not None and self._monitor.is_alive():
            self._monitor.join()
        self._terminate_owned()

    def open_editor(self):
        state = self.snapshot()
        if state["state"] != "ready":
            raise ValueError("Start or reuse a ready MiniHou app first")
        if not state["editor_url"]:
            raise ValueError("The Rust editor is in the MiniHou app window")
        return webbrowser.open(state["editor_url"])
