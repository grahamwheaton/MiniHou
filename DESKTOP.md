# MiniHou desktop

Double-click `MiniHou.vbs` in this checkout to open MiniHou in its own window, with no terminal or browser tabs. It uses the bundled development runtime under build/runtime/python312. `Start-MiniHou-Live.cmd` also opens the desktop window and keeps a diagnostic console.

For distribution, install the desktop extra (`pip install ".[desktop]"`) and PyInstaller, then run `python tools/build_app.py --no-smoke`. The resulting MiniHou folder contains MiniHou.exe, the runtime, editor, examples and Blender add-on. Distribute the whole folder or ZIP, not just the EXE. Windows requires Microsoft Edge WebView2 Runtime.

The editor saves into the workspace. Closing the window waits for submitted edits and saves the graph before stopping its engine. Blender modifiers keep their last cooked geometry while MiniHou is closed. Launching again without a graph argument reuses a running desktop window for that workspace. Simultaneous launches during initial startup can still create separate windows.

Blender Edit Network selects the modifier graph and raises the desktop window. Restart Blender after installing the updated add-on. Older browser engines retain browser fallback; close those manually if Blender is still connected to an older session.

Advanced options: `--browser` opens a browser; `--no-browser` runs only the engine. Ports are selected automatically when occupied. Windowed launcher diagnostics are written to `%LOCALAPPDATA%/MiniHou/app.log`.

Validation: `python tools/test_worktree.py tests.test_desktop tests.test_network_editor tests.test_blender_setup tests.test_editor_e2e`. `python tools/test_desktop_native.py` opens a temporary native Windows test window, checks editor/canvas and reuse, then closes it.

GPU: on laptops with an integrated and a discrete GPU the desktop window asks WebView2 for the fast one (`--force_high_performance_gpu`; the viewport was about 4x slower on the integrated GPU at 4M quads). Set `MINIHOU_GPU=default` to keep the system's choice, or set `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` yourself, which is kept.
