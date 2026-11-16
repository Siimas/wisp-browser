#!/usr/bin/env python3
"""Wisp performance budget check (Python standard library only).

    cargo build --release
    python3 bench/perf_check.py                    # measure and compare with bench/budget.json
    python3 bench/perf_check.py --size-only        # binary size only (any system)
    python3 bench/perf_check.py --base old.json    # also show the change against an earlier run

Exits 1 when a metric is over its budget. On macOS it starts the release build
`--runs` times on a clean throwaway profile with one local page, in two layouts:
the bar as it ships, and the bar collapsed (which also builds the floating copy of
the interface). Each run records:

- launch: from start until the page's `load` event reaches this script's server
- memory: physical footprint (Activity Monitor's "Memory") of the whole process
  tree, WebKit's helper processes included, after the page has settled
- idle CPU: CPU time of that tree over `--idle` seconds, as % of one core

Reported values are medians over the runs. Every instance is stopped by its own
process id; nothing else on the machine is touched and nothing leaves it.
"""
import argparse, collections, ctypes, http.server, json, os, pathlib, plistlib, shutil
import signal, statistics, subprocess, sys, tempfile, threading, time

REPO = pathlib.Path(__file__).resolve().parent.parent
EXE = REPO / "target/release" / ("wisp.exe" if os.name == "nt" else "wisp")
SETTLE = 10  # seconds after the page loaded before memory is sampled
LAYOUTS = {"": {}, "_collapsed": {"layout": {"collapsed": True}}}

# ------------------------------------------------------------------ macOS processes
# How the tree is found and measured is the same as bench/bench.py (docs/PERFORMANCE.md).
if sys.platform == "darwin":
    libc = ctypes.CDLL(None)

    class Timebase(ctypes.Structure):
        _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]

    _tb = Timebase()
    libc.mach_timebase_info(ctypes.byref(_tb))


def rusage(pid):
    """(cpu seconds, physical footprint bytes) of one process, or None if it is gone."""
    buf = (ctypes.c_uint64 * 20)()  # struct rusage_info_v2: 16-byte uuid, then 18 counters
    if libc.proc_pid_rusage(pid, 2, ctypes.byref(buf)) != 0:
        return None
    return (buf[2] + buf[3]) * _tb.numer / _tb.denom / 1e9, buf[9]


def ps():
    out = subprocess.run(["ps", "-axo", "pid=,ppid=,comm="], capture_output=True, text=True).stdout
    rows = (line.split(None, 2) for line in out.splitlines())
    return [(int(r[0]), int(r[1]), r[2]) for r in rows if len(r) == 3]


def tree(main):
    """{pid: executable}: descendants of `main` plus every process it is responsible for
    (WebKit's WebContent, GPU and Networking helpers are children of launchd)."""
    rows, kids = ps(), collections.defaultdict(list)
    for pid, ppid, _ in rows:
        kids[ppid].append(pid)
    mine, stack = set(), [main]
    while stack:
        p = stack.pop()
        if p not in mine:
            mine.add(p)
            stack += kids[p]
    mine |= {pid for pid, _, _ in rows if libc.responsibility_get_pid_responsible_for_pid(pid) == main}
    return {pid: comm for pid, _, comm in rows if pid in mine}


def sample(main):
    return {pid: r for pid in tree(main) if (r := rusage(pid))}


def memory_mb(main):
    """Median of three samples two seconds apart."""
    totals = []
    for _ in range(3):
        totals.append(sum(r[1] for r in sample(main).values()) / 2**20)
        time.sleep(2)
    return statistics.median(totals)


def idle_cpu(main, seconds):
    a, t0 = sample(main), time.monotonic()
    time.sleep(seconds)
    b, dt = sample(main), time.monotonic() - t0
    # Processes that came or went in the window are counted from/to zero.
    return 100 * sum(r[0] - a.get(pid, (0, 0))[0] for pid, r in b.items()) / dt


# ------------------------------------------------------------------ local page
class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.loaded, self.cond = {}, threading.Condition()
        threading.Thread(target=self.serve_forever, daemon=True).start()

    def wait(self, token, timeout):
        with self.cond:
            if not self.cond.wait_for(lambda: token in self.loaded, timeout):
                raise TimeoutError(f"the page did not load within {timeout}s")
            return self.loaded[token]


PAGE = """<!doctype html><meta charset=utf-8><title>Simple page</title>
<style>body{font:16px/1.5 system-ui;max-width:40em;margin:3em auto}</style>
<h1>Simple page</h1>%s<script>addEventListener('load',()=>fetch('/done?t=%s'))</script>"""


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        path, _, token = self.path.partition("?t=")
        body = b""
        if path == "/page":
            body = (PAGE % ("<p>The quick brown fox jumps over the lazy dog.</p>" * 20, token)).encode()
        elif path == "/done":
            with self.server.cond:
                self.server.loaded[token] = time.monotonic()
                self.server.cond.notify_all()
        self.send_response(200 if body else 204)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


# ------------------------------------------------------------------ one run
def bundle(tmp):
    """A minimal app bundle around the release binary. Started through `open`, the app is
    its own responsible process, which is what ties WebKit's helpers to it."""
    app = tmp / "Wisp.app"
    (app / "Contents/MacOS").mkdir(parents=True)
    shutil.copy2(EXE, app / "Contents/MacOS/wisp")
    info = {"CFBundleExecutable": "wisp", "CFBundleIdentifier": "io.github.siimas.wisp.perfcheck",
            "CFBundleName": "Wisp", "CFBundlePackageType": "APPL", "NSHighResolutionCapable": True}  # fmt: skip
    (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
    return app


def run(app, server, profile, config, token, idle):
    cfg = profile / "Library/Application Support/Wisp"
    cfg.mkdir(parents=True)
    (cfg / "config.json").write_text(json.dumps(config))
    url = f"http://127.0.0.1:{server.server_address[1]}/page?t={token}"
    (cfg / "session.json").write_text(json.dumps({"workspace": 1, "tabs": [{"url": url, "active": True}]}))
    exe = str(app / "Contents/MacOS/wisp")
    t0 = time.monotonic()
    # HOME moves Wisp's own files; CFFIXED_USER_HOME moves WebKit's data with them.
    subprocess.run(["open", "-n", "-g", "--env", f"HOME={profile}", "--env", f"CFFIXED_USER_HOME={profile}", str(app)], check=True)  # fmt: skip
    main = None
    while main is None and time.monotonic() - t0 < 30:  # the bundle path is ours alone
        main = next((pid for pid, _, comm in ps() if comm == exe), None)
        time.sleep(0.02)
    if main is None:
        raise RuntimeError("Wisp did not start")
    try:
        launch = server.wait(token, 60) - t0
        if not idle:  # warm-up run
            return {}
        time.sleep(SETTLE)
        return {"launch_s": launch, "memory_mb": memory_mb(main), "idle_cpu_pct": idle_cpu(main, idle)}
    finally:
        stop(main)


def stop(main):
    """Stop the instance we started, by process id only."""
    pids = tree(main)
    try:
        os.kill(main, signal.SIGTERM)
    except ProcessLookupError:
        raise RuntimeError("Wisp exited by itself during the measurement") from None
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline and any(pid == main for pid, _, _ in ps()):
        time.sleep(0.25)
    time.sleep(1)
    now = {pid: comm for pid, _, comm in ps()}
    for pid, comm in pids.items():  # stragglers: same pid, same executable, still ours
        if now.get(pid) == comm and pid != 1:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


# ------------------------------------------------------------------ report
def measure(args):
    out = {"binary_kb": [EXE.stat().st_size / 1024]}
    if args.size_only:
        return out
    if sys.platform != "darwin":
        sys.exit("runtime metrics are macOS only; use --size-only")
    server = Server()
    with tempfile.TemporaryDirectory(prefix="wisp-perf-") as tmp:
        tmp = pathlib.Path(tmp).resolve()  # /var is /private/var; ps reports the real path
        app = bundle(tmp)
        # Unrecorded: the first launch on a machine fills system caches (it was 2x slower
        # and 30% heavier than the next ones on the CI runner).
        run(app, server, tmp / "warmup", {}, "warmup", 0)
        for i in range(args.runs):
            for suffix, config in LAYOUTS.items():  # interleaved, so drift hits both alike
                name = f"run{i}{suffix}"
                r = run(app, server, tmp / name, config, name, args.idle)
                print(f"{name}: " + ", ".join(f"{k} {v:.2f}" for k, v in r.items()), flush=True)
                for k, v in r.items():
                    out.setdefault(k + suffix, []).append(v)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--idle", type=int, default=20, help="idle CPU window, seconds")
    ap.add_argument("--size-only", action="store_true")
    ap.add_argument("--budget", type=pathlib.Path, default=REPO / "bench/budget.json")
    ap.add_argument("--base", type=pathlib.Path, help="an earlier --out file to compare with")
    ap.add_argument("--out", type=pathlib.Path, help="write the measured runs here (JSON)")
    args = ap.parse_args()

    runs = measure(args)
    if args.out:
        args.out.write_text(json.dumps(runs, indent=1) + "\n")
    budget = json.loads(args.budget.read_text())["max"]
    base = json.loads(args.base.read_text()) if args.base and args.base.exists() else {}

    print(f"\n{'metric':<26}{'median':>9}  {'runs':<22}{'base':>9}{'change':>9}{'budget':>9}")
    failed = []
    for k, vals in runs.items():
        med, limit = statistics.median(vals), budget.get(k)
        b = statistics.median(base[k]) if base.get(k) else None
        change = f"{100 * (med - b) / b:+.0f}%" if b else ""
        over = limit is not None and med > limit
        failed += [k] * over
        spread = " ".join(f"{v:.1f}" for v in vals) if len(vals) > 1 else ""
        print(f"{k:<26}{med:>9.2f}  {spread:<22}{(f'{b:.2f}' if b else '-'):>9}{change:>9}"
              f"{(f'{limit:g}' if limit is not None else '-'):>9}  {'OVER BUDGET' if over else 'ok'}")  # fmt: skip
    if failed:
        print(f"\nOver budget: {', '.join(failed)}. If the increase is intended, raise the number in "
              f"{args.budget.relative_to(REPO)} and say why in the pull request (CONTRIBUTING.md).")  # fmt: skip
        sys.exit(1)


if __name__ == "__main__":
    main()
