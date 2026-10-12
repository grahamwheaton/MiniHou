"""Putting the add-on into Blender: finding installs, the zip, installing, launching, and the packed app's workspace."""
import importlib.util
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
import zipfile
from io import BytesIO
from pathlib import Path

from minihou import app, blender_setup

ROOT = Path(__file__).resolve().parent.parent
HAS_BPY = importlib.util.find_spec("bpy") is not None


class TempCase(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="minihou_bs_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        old = os.environ.get("MINIHOU_HOME")
        os.environ["MINIHOU_HOME"] = str(self.root / "home")
        self.addCleanup(lambda: os.environ.__setitem__("MINIHOU_HOME", old) if old is not None else os.environ.pop("MINIHOU_HOME", None))


class DiscoveryTests(TempCase):
    def exe(self, *parts):
        p = self.root.joinpath(*parts)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text("x", encoding="utf-8")
        return str(p)

    def test_installs_are_listed_newest_first_with_versions_and_support(self):
        a = self.exe("Program Files", "Blender Foundation", "Blender 4.1", "blender.exe")
        b = self.exe("Program Files", "Blender Foundation", "Blender 5.1", "blender.exe")
        got = blender_setup.list_blenders("windows", {"ProgramFiles": str(self.root / "Program Files")}, lambda n: None)
        self.assertEqual([g["path"] for g in got], [b, a])
        self.assertEqual([g["version"] for g in got], ["5.1", "4.1"])
        self.assertEqual([g["supported"] for g in got], [True, False])                     # the add-on needs 4.2+

    def test_nothing_installed_is_an_empty_list(self):
        self.assertEqual(blender_setup.list_blenders("windows", {}, lambda n: None), [])

    def test_blender_exe_variable_is_listed_first(self):
        mine = self.exe("tools", "blender.exe")
        got = blender_setup.list_blenders("windows", {"BLENDER_EXE": mine}, lambda n: None)
        self.assertEqual(got[0]["path"], mine)
        self.assertIsNone(got[0]["version"])                                               # unknown, not guessed wrongly


class ZipTests(TempCase):
    def test_zip_has_the_addon_folder_at_the_top_and_matches_the_source(self):
        data = blender_setup.build_addon_zip()
        z = zipfile.ZipFile(BytesIO(data))
        names = z.namelist()
        self.assertIn("minihou_link/__init__.py", names)
        self.assertTrue(all(n.startswith("minihou_link/") for n in names))
        self.assertFalse([n for n in names if "__pycache__" in n or n.endswith(".pyc")])
        for n in names:
            self.assertEqual(z.read(n), (ROOT / "blender_addon" / n).read_bytes(), n)
        self.assertEqual(blender_setup.build_addon_zip(), data)                            # reproducible: the same bytes every time

    def test_a_missing_addon_is_a_clear_error_not_a_crash(self):
        old = os.environ.get("MINIHOU_ADDON_DIR")
        os.environ["MINIHOU_ADDON_DIR"] = str(self.root / "nowhere")
        orig = blender_setup.addon_source
        try:
            blender_setup.addon_source = lambda: None
            with self.assertRaises(FileNotFoundError):
                blender_setup.build_addon_zip()
            self.assertFalse(blender_setup.install_addon("irrelevant")["ok"])
        finally:
            blender_setup.addon_source = orig
            if old is None:
                os.environ.pop("MINIHOU_ADDON_DIR")
            else:
                os.environ["MINIHOU_ADDON_DIR"] = old


@unittest.skipIf(os.name == "nt", "the stand-in Blender is a shell script")
class RunningBlenderTests(TempCase):
    def fake(self, body):
        p = self.root / "blender"
        p.write_text("#!/bin/sh\n" + body, encoding="utf-8")
        p.chmod(p.stat().st_mode | stat.S_IEXEC)
        return str(p)

    def test_install_reports_what_blender_wrote(self):
        # the script writes the result file named by the last argument, as the real install script does
        exe = self.fake('out="$6"; echo \'{"ok": true, "blender": "5.1.0", "path": "/x/minihou_link"}\' > "$out"\n')
        r = blender_setup.install_addon(exe)
        self.assertTrue(r["ok"], r)
        self.assertIn("5.1.0", r["message"])
        self.assertIn("restart", r["message"])                                             # an open Blender must be restarted

    def test_install_failures_are_messages_not_exceptions(self):
        r = blender_setup.install_addon(self.fake("echo boom >&2\nexit 1\n"))
        self.assertFalse(r["ok"])
        self.assertIn("boom", r["message"])
        r = blender_setup.install_addon(self.fake('echo \'{"ok": false, "error": "no write access"}\' > "$6"\n'))
        self.assertEqual((r["ok"], r["message"]), (False, "no write access"))
        self.assertFalse(blender_setup.install_addon(str(self.root / "missing"))["ok"])

    def test_launch_gives_blender_a_bootstrap_with_the_engines_port(self):
        log = self.root / "args.txt"
        exe = self.fake(f'echo "$@" > {log}\n')
        blender_setup.launch_blender(exe, 4321).wait(10)
        args = log.read_text(encoding="utf-8").split()
        self.assertIn("--python", args)
        self.assertEqual(args[args.index("--port") + 1], "4321")
        boot = Path(args[args.index("--python") + 1])
        self.assertTrue(boot.is_file())
        self.assertTrue((boot.parent / "minihou_link" / "__init__.py").is_file())          # a private copy of the add-on next to it
        compile(boot.read_text(encoding="utf-8"), "bootstrap.py", "exec")


@unittest.skipUnless(HAS_BPY, "needs the bpy module")
class RealBlenderInstallTests(TempCase):
    def test_the_install_script_unpacks_and_enables_the_addon(self):
        scripts = self.root / "scripts"
        z, out, script = self.root / "a.zip", self.root / "out.json", self.root / "install.py"
        z.write_bytes(blender_setup.build_addon_zip())
        script.write_text(blender_setup.INSTALL_SCRIPT, encoding="utf-8")
        env = dict(os.environ, BLENDER_USER_SCRIPTS=str(scripts), HOME=str(self.root), XDG_CONFIG_HOME=str(self.root / "cfg"),
                   APPDATA=str(self.root / "appdata"))
        code = ("import sys; sys.argv = ['blender', '--', %r, %r]\nexec(compile(open(%r, encoding='utf-8').read(), 'install.py', 'exec'))"
                % (str(z), str(out), str(script)))
        r = subprocess.run([sys.executable, "-c", code], env=env, capture_output=True, text=True, timeout=240, encoding="utf-8")
        res = json.loads(out.read_text(encoding="utf-8")) if out.exists() else {}
        self.assertTrue(res.get("ok"), (res, r.stdout[-400:], r.stderr[-400:]))
        self.assertTrue((scripts / "addons" / "minihou_link" / "__init__.py").is_file())


class WorkspaceTests(TempCase):
    def test_first_run_creates_folders_examples_and_a_welcome_graph(self):
        ws = self.root / "MiniHou"
        graph = app.prepare_workspace(ws)
        self.assertEqual(graph, ws / "graphs" / "welcome.json")
        self.assertTrue((ws / "nodes").is_dir())
        self.assertTrue((ws / "examples" / "crater.json").is_file())
        self.assertIn('"nodes"', graph.read_text(encoding="utf-8"))

    def test_second_run_never_overwrites_the_users_work(self):
        ws = self.root / "MiniHou"
        graph = app.prepare_workspace(ws)
        graph.write_text('{"version": 1, "nodes": {"mine": {"type": "box"}}}', encoding="utf-8")
        (ws / "examples" / "crater.json").write_text("edited", encoding="utf-8")
        again = app.prepare_workspace(ws)
        self.assertEqual(again, graph)
        self.assertIn("mine", graph.read_text(encoding="utf-8"))
        self.assertEqual((ws / "examples" / "crater.json").read_text(encoding="utf-8"), "edited")

    def test_an_empty_graph_is_made_when_there_are_no_examples(self):
        orig = app.examples_source
        try:
            app.examples_source = lambda: None
            g = app.prepare_workspace(self.root / "bare")
        finally:
            app.examples_source = orig
        self.assertEqual(json.loads(g.read_text(encoding="utf-8"))["nodes"], {})

    def test_the_program_starts_serves_the_editor_and_the_zip(self):
        ws = self.root / "ws"
        env = dict(os.environ, PYTHONUNBUFFERED="1", MINIHOU_HOME=str(self.root / "home"))
        log = self.root / "out.log"
        with open(log, "w", encoding="utf-8") as lf:
            p = subprocess.Popen([sys.executable, "-m", "minihou", "app", "--no-browser", "--workspace", str(ws), "--http-port", "0",
                                  "--port", "0"], stdout=lf, stderr=subprocess.STDOUT, env=env, cwd=str(ROOT))
        self.addCleanup(lambda: (p.terminate(), p.wait(15)))
        url = None
        end = time.monotonic() + 60
        while time.monotonic() < end and not url:
            for line in log.read_text(encoding="utf-8", errors="replace").splitlines():
                if line.startswith("editor: "):
                    url = line.split(": ", 1)[1].strip()
            time.sleep(0.2)
        self.assertTrue(url, log.read_text(encoding="utf-8", errors="replace"))
        import urllib.request
        data = urllib.request.urlopen(url + "addon/minihou_link.zip", timeout=20).read()
        self.assertIn("minihou_link/__init__.py", zipfile.ZipFile(BytesIO(data)).namelist())
        self.assertTrue((ws / "graphs" / "welcome.json").is_file())


if __name__ == "__main__":
    unittest.main()
