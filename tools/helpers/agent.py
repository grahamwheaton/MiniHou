#!/usr/bin/env python3
"""Detached worker launcher for codex / agy. No dependencies, no server.

  agent.py start codex|agy <prompt-file|-> [--cwd DIR] [--model M] [--effort E] [--name N]
  agent.py dash [--html] | clear [--all] | list | status [ID] | tail ID [N] | result ID | send ID "text" | cancel ID

State lives in ~/.multi-agent/<repo-name>/<id>/ on the machine running the agents:
  task.json (immutable), status.json (atomic), inbox.jsonl (append-only), worker.log, prompt.txt

Follow-ups: neither CLI accepts input while a turn runs, so `send` appends to the
inbox and the supervisor delivers it at the next checkpoint (end of the running turn)
with `codex exec resume <session>`; agy uses --continue. Max 3 resume rounds per task.
"""
import io, json, os, re, shutil, subprocess, sys, time, traceback, uuid
from pathlib import Path

ROOT = Path.home() / ".multi-agent"
LIMIT_RE = re.compile(r"usage limit|rate limit|quota|too many requests|try again (at|in)", re.I)
MAX_ROUNDS = 3


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%S")


def repo_name(cwd):
    p = Path(cwd).resolve()
    for q in [p, *p.parents]:
        if (q / ".git").exists():
            # worktrees keep a .git file; use the main repo folder name when possible
            return q.name
    return p.name


def tdir(tid):
    hits = list(ROOT.glob(f"*/{tid}"))
    if not hits:
        sys.exit(f"no such task {tid}")
    return hits[0]


def jread(p, default=None):
    try:
        return json.loads(Path(p).read_text(encoding="utf-8"))
    except Exception:
        return default


def set_status(d, **kw):
    s = jread(d / "status.json", {})
    s.update(kw, updated=now())
    tmp = d / f"status.{os.getpid()}.tmp"
    tmp.write_text(json.dumps(s), encoding="utf-8")
    os.replace(tmp, d / "status.json")  # atomic


def pid_alive(pid):
    if not pid:
        return False
    if os.name == "nt":
        out = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH"], capture_output=True, text=True).stdout
        return str(pid) in out
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def build_cmd(t, prompt, resume_sid=None):
    if t["agent"] == "codex":
        base = ["codex", "exec"]
        if resume_sid:
            base += ["resume", resume_sid]
        base += (["-m", t["model"]] if t["model"] else []) + ["-c", f"model_reasoning_effort={t['effort']}",
                 "--dangerously-bypass-approvals-and-sandbox", "-"]
        return base, prompt
    # agy: prompt attached to -p=, double quotes break its arg parsing
    cmd = ["agy", "--model", t["model"], "--dangerously-skip-permissions"]
    if resume_sid == "continue":
        cmd.append("--continue")
    cmd.append("-p=" + prompt.replace('"', "'"))
    return cmd, None


def run_turn(d, t, prompt, resume_sid):
    cmd, stdin = build_cmd(t, prompt, resume_sid)
    cmd[0] = shutil.which(cmd[0]) or cmd[0]  # .cmd shims on Windows
    with open(d / "worker.log", "ab") as log:
        log.write(f"\n=== {now()} turn start ===\n".encode())
        log.flush()
        p = subprocess.Popen(cmd, cwd=t["cwd"], stdin=subprocess.PIPE if stdin else subprocess.DEVNULL,
                             stdout=log, stderr=subprocess.STDOUT)
        set_status(d, child=p.pid)
        if stdin:
            p.stdin.write(stdin.encode())
            p.stdin.close()
        while p.poll() is None:
            if (d / "cancel").exists():
                p.kill()
                return "cancelled"
            time.sleep(1)
        return "ok" if p.returncode == 0 else f"exit {p.returncode}"


def supervise(tid):
    d = tdir(tid)
    t = jread(d / "task.json")
    set_status(d, state="running", pid=os.getpid(), started=now(), rounds=0)
    prompt = (d / "prompt.txt").read_text(encoding="utf-8")
    sid = "continue" if t["agent"] == "agy" else None
    acked = 0
    for rnd in range(MAX_ROUNDS + 1):
        outcome = run_turn(d, t, prompt, sid if rnd else None)
        log = (d / "worker.log").read_bytes().decode("utf-8", "replace")
        if rnd == 0 and t["agent"] == "codex":
            m = re.search(r"session id:\s*([0-9a-f-]{36})", log, re.I)
            sid = m.group(1) if m else None
            set_status(d, session=sid)
        if outcome == "cancelled":
            return set_status(d, state="cancelled", finished=now())
        if LIMIT_RE.search(log[-4000:]) and outcome != "ok":
            return set_status(d, state="blocked", reason="usage limit suspected; not retrying", finished=now())
        if outcome != "ok":
            return set_status(d, state="failed", reason=outcome, finished=now())
        msgs = [json.loads(l) for l in (d / "inbox.jsonl").read_text(encoding="utf-8").splitlines() if l.strip()] \
            if (d / "inbox.jsonl").exists() else []
        new = msgs[acked:]
        if not new or not sid or rnd == MAX_ROUNDS:
            note = "unsent follow-ups" if new else None
            return set_status(d, state="completed", finished=now(), rounds=rnd, **({"note": note} if note else {}))
        acked = len(msgs)
        prompt = "Follow-up instructions from the controller (apply them to your work, then reply briefly):\n" + \
                 "\n".join(f"- {m['text']}" for m in new)
        set_status(d, state="running", rounds=rnd + 1, delivered=acked)


def cmd_start(a):
    agent, pf = a[0], a[1]
    opt = {"--cwd": os.getcwd(), "--model": None, "--effort": "medium", "--name": None}
    for i in range(2, len(a) - 1, 2):
        opt[a[i]] = a[i + 1]
    model = opt["--model"] or (None if agent == "codex" else "gemini-3.8-flash-medium")  # codex: CLI default
    prompt = sys.stdin.read() if pf == "-" else Path(pf).read_text(encoding="utf-8")
    cwd = str(Path(opt["--cwd"]).resolve())
    tid = (opt["--name"] or agent) + "-" + uuid.uuid4().hex[:6]
    d = ROOT / repo_name(cwd) / tid
    d.mkdir(parents=True)
    (d / "prompt.txt").write_text(prompt, encoding="utf-8")
    (d / "task.json").write_text(json.dumps(dict(id=tid, agent=agent, model=model, effort=opt["--effort"],
                                                  cwd=cwd, created=now())), encoding="utf-8")
    set_status(d, state="queued")
    flags = 0x00000008 | 0x00000200 | 0x08000000 if os.name == "nt" else 0  # DETACHED|NEW_GROUP|NO_WINDOW
    subprocess.Popen([sys.executable, __file__, "_run", tid], creationflags=flags,
                     start_new_session=(os.name != "nt"), stdin=subprocess.DEVNULL,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, close_fds=True)
    print(tid)


def summary(d):
    s = jread(d / "status.json", {})
    t = jread(d / "task.json", {})
    if s.get("state") == "running" and not pid_alive(s.get("pid")):
        s["state"] = "died"  # supervisor vanished; log + session id preserved for a manual resume
    pend = sum(1 for _ in open(d / "inbox.jsonl")) - s.get("delivered", 0) if (d / "inbox.jsonl").exists() else 0
    return f"{t.get('id')} [{d.parent.name}] {t.get('agent')} {s.get('state')} rounds={s.get('rounds', 0)} " \
           f"pending={pend} {s.get('reason') or s.get('note') or ''}".rstrip()


def rows():
    out = []
    for d in sorted(ROOT.glob("*/*"), key=lambda p: p.stat().st_mtime):
        if d.parent.name == "archive" or not d.is_dir() or not (d / "task.json").exists():
            continue
        t, s = jread(d / "task.json", {}), jread(d / "status.json", {})
        state = s.get("state")
        if state == "running" and not pid_alive(s.get("pid")):
            state = "died"
        task = ((d / "prompt.txt").read_text(encoding="utf-8", errors="replace").strip().splitlines()[:1] or [""])[0][:60]
        log = (d / "worker.log").read_text(encoding="utf-8", errors="replace") if (d / "worker.log").exists() else ""
        used = re.findall(r"tokens used\s*([\d,]+)", log)
        out.append((t.get("id"), t.get("agent"), task, state, (used[-1] if used else "-"), s.get("updated", "")[11:]))
    return out


def codex_usage():
    """Latest rate-limit snapshot Codex itself recorded (same numbers as /status)."""
    files = sorted((Path.home() / ".codex" / "sessions").rglob("*.jsonl"), key=lambda p: p.stat().st_mtime)
    for f in reversed(files[-5:]):
        hits = re.findall(r'"primary":\{"used_percent":([\d.]+),"window_minutes":(\d+),"resets_at":(\d+)\},'
                          r'"secondary":\{"used_percent":([\d.]+),"window_minutes":(\d+),"resets_at":(\d+)',
                          f.read_text(encoding="utf-8", errors="replace"))
        if hits:
            a, _, ar, b, _, br = hits[-1]
            t = lambda x: time.strftime("%a %H:%M", time.localtime(int(x)))
            return f"Codex: 5h window {float(a):.0f}% used (resets {t(ar)}) | weekly {float(b):.0f}% used (resets {t(br)})"
    return "Codex: no usage snapshot found"



def extra_lines():
    """Rows the dash cannot read from local job files: Claude, Opus/Sonnet subagents (kept by hand), and Bob."""
    out = []
    claude = ROOT / "claude_usage.txt"
    out.append("Claude (coordinator): " + (claude.read_text(encoding="utf-8").strip() if claude.exists() else "usage not recorded"))
    manual = ROOT / "subagents.txt"
    if manual.exists():
        for line in manual.read_text(encoding="utf-8").splitlines():
            if line.strip() and not line.startswith("#"):
                out.append("Subagent: " + line.strip())
    try:
        import subprocess as sp
        r = sp.run(["ssh", "-i", str(Path.home() / ".ssh/id_ed25519_minihou"), "-o", "BatchMode=yes", "-o", "ConnectTimeout=5",
                    "graham@100.84.79.93", "dir /b C:/Users/Graham/minihou_bench"], capture_output=True, text=True, timeout=12)
        out.append("Bob (work PC): " + ("reachable, " + str(len([l for l in r.stdout.split() if l.endswith('-wt')])) + " worktrees" if r.returncode == 0 else "unreachable"))
    except Exception:
        out.append("Bob (work PC): unreachable")
    return out

def dash(html):
    hdr = ("agent/id", "cli", "task", "status", "tokens", "updated")
    r = rows()
    note = codex_usage() + " | agy: no CLI usage readout"
    if not html:
        w = [max(len(str(x[i])) for x in [hdr, *r]) for i in range(6)]
        for x in [hdr, *r]:
            print("  ".join(str(v).ljust(w[i]) for i, v in enumerate(x)))
        print(note)
        for line in extra_lines(): print(line)
        return
    col = {"running": "#d29922", "completed": "#3fb950", "failed": "#f85149", "died": "#f85149", "blocked": "#f85149"}
    body = "".join("<tr>" + "".join(f"<td style='color:{col.get(v, '')}'>{v}</td>" if i == 3 else f"<td>{v}</td>"
                                    for i, v in enumerate(x)) + "</tr>" for x in r)
    page = ("<meta charset=utf-8><meta http-equiv=refresh content=5><title>Agents</title>"
            "<body style='font:14px sans-serif;background:#161b22;color:#c9d1d9;padding:16px'><table cellpadding=6>"
            f"<tr>{''.join(f'<th align=left>{h}</th>' for h in hdr)}</tr>{body}</table><p>{note}</p>")
    f = ROOT / "dash.html"
    f.write_text(page, encoding="utf-8")
    print(f)


def clear(states):
    """Move finished tasks out of the dash into ~/.multi-agent/archive (logs kept)."""
    n = 0
    for d in list(ROOT.glob("*/*")):
        if d.is_dir() and d.parent.name != "archive" and (d / "task.json").exists() and summary(d).split()[3] in states:
            dest = ROOT / "archive" / d.parent.name
            dest.mkdir(parents=True, exist_ok=True)
            d.rename(dest / d.name)
            n += 1
    print(f"archived {n}")


def main():
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
    c, a = sys.argv[1], sys.argv[2:]
    if c == "clear":
        return clear({"died", "failed", "cancelled", "blocked"} | ({"completed"} if "--all" in a else set()))
    if c == "dash":
        return dash("--html" in a)
    if c == "_run":
        try:
            return supervise(a[0])
        except Exception:
            d = tdir(a[0])
            (d / "worker.log").open("a").write(traceback.format_exc())
            return set_status(d, state="failed", reason="supervisor crashed", finished=now())
    if c == "start":
        return cmd_start(a)
    if c in ("list", "status") and not a:
        for d in sorted(ROOT.glob("*/*"), key=lambda p: p.stat().st_mtime):
            if d.is_dir():
                print(summary(d))
        return
    d = tdir(a[0])
    if c == "status":
        print(summary(d))
    elif c == "tail":
        n = int(a[1]) if len(a) > 1 else 30
        print("\n".join((d / "worker.log").read_text(encoding="utf-8", errors="replace").splitlines()[-n:]))
    elif c == "result":
        # last block of the log after the final turn marker = the agent's closing message
        log = (d / "worker.log").read_text(encoding="utf-8", errors="replace")
        print(log.split("=== ")[-1][-3000:])
    elif c == "send":
        with open(d / "inbox.jsonl", "a", encoding="utf-8") as f:
            f.write(json.dumps(dict(id=uuid.uuid4().hex[:8], ts=now(), text=a[1])) + "\n")
        print("queued; delivered at next checkpoint (end of current turn)")
    elif c == "cancel":
        (d / "cancel").write_text("1")
        print("cancel requested")
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
