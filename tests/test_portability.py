"""Portability guards that matter on Windows but can be checked anywhere."""
import ast
import importlib.util
import os
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCES = [p for d in ("minihou", "blender_addon", "tools", "bench", "tests") for p in sorted((ROOT / d).rglob("*.py"))]


def _calls(tree):
    for n in ast.walk(tree):
        if isinstance(n, ast.Call):
            f = n.func
            yield n, (f.attr if isinstance(f, ast.Attribute) else f.id if isinstance(f, ast.Name) else ""), {k.arg for k in n.keywords}


class EncodingTests(unittest.TestCase):
    """Windows' default text encoding is the ANSI code page (cp1252), not UTF-8: every text I/O must say which."""

    def test_text_io_always_names_an_encoding(self):
        problems = []
        for p in SOURCES:
            tree = ast.parse(p.read_text(encoding="utf-8"))
            for n, name, kws in _calls(tree):
                where = f"{p.relative_to(ROOT)}:{n.lineno}"
                if name in ("read_text", "write_text") and "encoding" not in kws:
                    problems.append(f"{where} {name}() without encoding")
                if name == "open" and isinstance(n.func, ast.Name):
                    mode = n.args[1].value if len(n.args) > 1 and isinstance(n.args[1], ast.Constant) else "r"
                    if "b" not in str(mode) and "encoding" not in kws:
                        problems.append(f"{where} open({mode!r}) without encoding")
                if name in ("Popen", "run", "check_output") and ("text" in kws or "universal_newlines" in kws) \
                        and "encoding" not in kws:
                    problems.append(f"{where} subprocess.{name}(text=True) without encoding")
        self.assertEqual(problems, [])

    def test_console_strings_are_ascii(self):
        """A non-ASCII character printed to a cp1252 Windows console can raise UnicodeEncodeError."""
        problems = []
        for p in SOURCES:
            if p.parts[-2] == "tests":
                continue
            tree = ast.parse(p.read_text(encoding="utf-8"))
            docs = set()
            for n in ast.walk(tree):
                if isinstance(n, (ast.Module, ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)) and n.body \
                        and isinstance(n.body[0], ast.Expr) and isinstance(n.body[0].value, ast.Constant):
                    docs.add(id(n.body[0].value))
            for n in ast.walk(tree):
                if isinstance(n, ast.Constant) and isinstance(n.value, str) and id(n) not in docs \
                        and any(ord(c) > 127 for c in n.value):
                    problems.append(f"{p.relative_to(ROOT)}:{n.lineno} {n.value[:40]!r}")
        self.assertEqual(problems, [])

    def test_engine_cli_survives_a_cp1252_console(self):
        import subprocess
        import sys
        env = {**os.environ, "PYTHONIOENCODING": "cp1252:strict", "PYTHONUTF8": "0"}
        for args in (["nodes"], ["cook", str(ROOT / "examples" / "graphs" / "demo.json"), "--repeat", "1"]):
            out = subprocess.run([sys.executable, "-m", "minihou"] + args, cwd=str(ROOT), env=env, capture_output=True,
                                 text=True, encoding="cp1252", errors="strict")
            self.assertEqual(out.returncode, 0, out.stderr)


def _load_launcher():
    spec = importlib.util.spec_from_file_location("launch_blender", ROOT / "tools" / "launch_blender.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class BlenderDiscoveryTests(unittest.TestCase):
    def setUp(self):
        self.lb = _load_launcher()
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def exe(self, *parts):
        p = self.root.joinpath(*parts)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(b"")
        return str(p)

    def candidates(self, env, which=lambda name: None):
        return list(self.lb.candidate_paths(platform="windows", env=env, which=which))

    def test_program_files_newest_version_first(self):
        pf = self.root / "Program Files"
        for v in ("4.2", "5.1", "4.10", "5.0"):
            self.exe("Program Files", "Blender Foundation", f"Blender {v}", "blender.exe")
        got = self.candidates({"ProgramFiles": str(pf)})
        versions = [Path(p).parent.name for p in got]
        self.assertEqual(versions, ["Blender 5.1", "Blender 5.0", "Blender 4.10", "Blender 4.2"])

    def test_explicit_env_var_and_path_win(self):
        self.exe("Program Files", "Blender Foundation", "Blender 5.1", "blender.exe")
        mine = self.exe("tools", "my blender", "blender.exe")
        on_path = self.exe("path", "blender.exe")
        got = self.candidates({"ProgramFiles": str(self.root / "Program Files"), "BLENDER_EXE": mine},
                              which=lambda name: on_path if name == "blender" else None)
        self.assertEqual(got[:2], [mine, on_path])

    def test_steam_and_per_user_installs(self):
        steam = self.exe("Program Files (x86)", "Steam", "steamapps", "common", "Blender", "blender.exe")
        user = self.exe("AppData", "Local", "Programs", "Blender Foundation", "Blender 5.1", "blender.exe")
        got = self.candidates({"ProgramFiles(x86)": str(self.root / "Program Files (x86)"),
                               "LOCALAPPDATA": str(self.root / "AppData" / "Local")})
        self.assertEqual(set(got), {steam, user})
        self.assertEqual(got[0], user)                      # versioned install sorts before the unversioned Steam one

    def test_nothing_found(self):
        self.assertEqual(self.candidates({}), [])
        self.assertIsNone(self.lb.find_blender(str(self.root / "missing.exe")))

    def test_find_blender_skips_stale_entries(self):
        real = self.exe("Program Files", "Blender Foundation", "Blender 5.1", "blender.exe")
        lb = self.lb
        orig = lb.candidate_paths
        try:
            lb.candidate_paths = lambda: iter([str(self.root / "gone" / "blender.exe"), real])
            self.assertEqual(lb.find_blender(), real)
        finally:
            lb.candidate_paths = orig


if __name__ == "__main__":
    unittest.main()
