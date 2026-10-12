import sys, threading, time, tempfile
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import webview
from minihou import app
from minihou.desktop import reuse
root=tempfile.mkdtemp(prefix='minihou_desktop_')
failures=[]
def check():
    try:
        for _ in range(120):
            time.sleep(.5)
            if webview.windows:
                w=webview.windows[0]
                try:
                    if w.evaluate_js('!!window.minihou'):
                        break
                except Exception: pass
        else: raise RuntimeError('Editor did not load')
        assert w.evaluate_js('!!document.querySelector("canvas")'), 'No viewport canvas'
        assert reuse(root), 'Existing window not reused'
        print('NATIVE DESKTOP PASS: editor loaded, canvas present, window reused',flush=True)
    except Exception as e:
        failures.append(e)
        print('NATIVE DESKTOP FAIL:',repr(e),flush=True)
    finally:
        w.confirm_close=False
        w.destroy()
threading.Thread(target=check,daemon=True).start()
app.main(['--workspace',root,'--port','0','--http-port','0'])

if failures: raise SystemExit(1)
