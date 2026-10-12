"""Interrupt Python cooking without terminating the engine or discarding edits."""
from functools import wraps
from pathlib import Path
import sys


class CookCancelled(BaseException):
    pass


ROOT = str(Path(__file__).resolve().parent)


def cancellable(fn):
    @wraps(fn)
    def run(session, *args, **kwargs):
        epoch = session._cancel_epoch
        document = session._document_id
        previous = sys.gettrace()
        counter = 0

        def trace(frame, event, arg):
            nonlocal counter
            filename = frame.f_code.co_filename.replace("\\", "/")
            if event == "call" and not (filename.startswith(ROOT.replace("\\", "/")) or filename.startswith("minihou/")):
                return None
            counter += 1
            if counter % 256 == 0:
                if epoch != session._cancel_epoch or document != session._document_id:
                    raise CookCancelled()
            return trace

        try:
            sys.settrace(trace)
            return fn(session, *args, **kwargs)
        except CookCancelled:
            if document == session._document_id:
                with session._idle:
                    session._cooked_rev = session._rev
                    session._idle.notify_all()
                session._report_error(None, "Cook cancelled (Escape)", session._rev)
            return {"ok": False, "error": {"message": "Cook cancelled (Escape)"}} if fn.__name__ != "cook_once" else None
        finally:
            sys.settrace(previous)
    return run
