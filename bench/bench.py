#!/usr/bin/env python3
"""Wisp benchmark harness (macOS, Python stdlib only). Results: docs/PERFORMANCE.md.

    cargo build --release && cargo packager --release --formats app
    python3 bench/bench.py size
    python3 bench/bench.py coldstart   [--runs 5]
    python3 bench/bench.py onetab      [--runs 3]
    python3 bench/bench.py tentabs     [--runs 3]      # memory + idle CPU
    python3 bench/bench.py speedometer --src <checkout of WebKit/Speedometer release/3.1>

    --browsers wisp,zen,chrome   which to run (default: all three)
    --foreground                 let the browser window come to the front
    --out FILE                   data file to merge into (default docs/performance-data.json)
    --wisp-app PATH              measure this Wisp.app instead of target/release/Wisp.app

Every browser runs as a separate instance on a throwaway profile under --tmp and is
stopped by process id. Nothing here touches a real profile or a browser already open.
Safari is not driven: it has no separate-profile launch, so only its size is recorded.

How things are measured, in one place:
- "page loaded" is the page itself calling back to this script's local server from
  its `load` event, so it is the same signal in every browser and needs no automation.
- A browser's processes are its process tree plus every process whose *responsible
  process* (what Activity Monitor groups by) is the browser. The second part is what
  catches WebKit's com.apple.WebKit.* helpers, which are children of launchd.
- Memory is read per process with proc_pid_rusage: `footprint` is phys_footprint
  (Activity Monitor's "Memory"), `rss` is resident size (double-counts shared pages).
"""
import argparse, collections, ctypes, http.server, json, os, pathlib, platform, shutil
import signal, statistics, subprocess, threading, time, urllib.parse

REPO = pathlib.Path(__file__).resolve().parent.parent
SITES = [  # ten-tab set: public front pages and articles, no sign-in
    "https://en.wikipedia.org/wiki/Web_browser",
    "https://github.com/WebKit/WebKit",
    "https://news.ycombinator.com/",
    "https://www.bbc.com/news",
    "https://www.theguardian.com/international",
    "https://stackoverflow.com/questions/tagged/rust",
    "https://developer.mozilla.org/en-US/docs/Web/JavaScript",
    "https://www.nytimes.com/",
    "https://www.amazon.com/",
    "https://www.youtube.com/",
]
OPEN_EVERY_MS = 4000  # Wisp opener: one new tab every 4 s
SETTLE_10 = 100  # seconds from launch to the ten-tab memory sample
ZEN_PREFS = {  # throwaway profile only: skip first-run pages, never touch the install
    "zen.welcome-screen.seen": True,
    "browser.aboutwelcome.enabled": False,
    "browser.shell.checkDefaultBrowser": False,
    "browser.startup.homepage_override.mstone": "ignore",
    "startup.homepage_welcome_url": "",
    "startup.homepage_welcome_url.additional": "",
    "datareporting.policy.dataSubmissionPolicyBypassNotification": True,
    "browser.sessionstore.resume_from_crash": False,
    "app.update.disabledForTesting": True,
}
BROWSERS = {
    "wisp": REPO / "target/release/Wisp.app/Contents/MacOS/wisp",
    "zen": pathlib.Path("/Applications/Zen.app/Contents/MacOS/zen"),
    "chrome": pathlib.Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
}

# ------------------------------------------------------------------ processes
libc = ctypes.CDLL(None)


class Timebase(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]


_tb = Timebase()
libc.mach_timebase_info(ctypes.byref(_tb))
RUSAGE_V2 = [  # struct rusage_info_v2 after the 16-byte uuid
    "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups", "pageins",
    "wired_size", "resident_size", "phys_footprint", "proc_start_abstime",
    "proc_exit_abstime", "child_user_time", "child_system_time", "child_pkg_idle_wkups",
    "child_interrupt_wkups", "child_pageins", "child_elapsed_abstime",
    "diskio_bytesread", "diskio_byteswritten",
]  # fmt: skip


def rusage(pid):
    buf = (ctypes.c_uint64 * (2 + len(RUSAGE_V2)))()
    if libc.proc_pid_rusage(pid, 2, ctypes.byref(buf)) != 0:
        return None
    return dict(zip(RUSAGE_V2, buf[2:]))


def ps(field):
    out = subprocess.run(["ps", "-axo", f"pid=,ppid=,{field}="], capture_output=True, text=True).stdout
    rows = (line.split(None, 2) for line in out.splitlines())
    return [(int(r[0]), int(r[1]), r[2]) for r in rows if len(r) == 3]


def tree(main):
    """{pid: executable} for the browser: descendants + processes it is responsible for."""
    rows = ps("comm")
    kids = collections.defaultdict(list)
    for pid, ppid, _ in rows:
        kids[ppid].append(pid)
    mine, stack = set(), [main]
    while stack:
        p = stack.pop()
        if p not in mine:
            mine.add(p)
            stack += kids[p]
    for pid, _, _ in rows:
        if libc.responsibility_get_pid_responsible_for_pid(pid) == main:
            mine.add(pid)
    return {pid: comm for pid, _, comm in rows if pid in mine}


def sample(main):
    procs = {}
    for pid, comm in tree(main).items():
        r = rusage(pid)
        if r:
            procs[pid] = {
                "name": os.path.basename(comm),
                "footprint": r["phys_footprint"],
                "rss": r["resident_size"],
                "cpu_s": (r["user_time"] + r["system_time"]) * _tb.numer / _tb.denom / 1e9,
                "wakeups": r["pkg_idle_wkups"] + r["interrupt_wkups"],
            }
    return procs


def mb(n):
    return round(n / 2**20, 1)


def memory(main):
    """Median of three samples two seconds apart, plus the per-kind breakdown of the last."""
    totals = []
    for _ in range(3):
        procs = sample(main)
        totals.append((sum(p["footprint"] for p in procs.values()), sum(p["rss"] for p in procs.values())))
        time.sleep(2)
    kinds = collections.defaultdict(lambda: {"count": 0, "footprint_mb": 0.0, "rss_mb": 0.0})
    for p in procs.values():
        k = kinds[p["name"]]
        k["count"] += 1
        k["footprint_mb"] = round(k["footprint_mb"] + mb(p["footprint"]), 1)
        k["rss_mb"] = round(k["rss_mb"] + mb(p["rss"]), 1)
    return {
        "footprint_mb": mb(statistics.median(t[0] for t in totals)),
        "rss_mb": mb(statistics.median(t[1] for t in totals)),
        "processes": len(procs),
        "by_kind": dict(kinds),
    }


def idle(main, seconds=60):
    a, t0 = sample(main), time.monotonic()
    time.sleep(seconds)
    b, dt = sample(main), time.monotonic() - t0
    # Processes that came or went during the window are counted from/to zero.
    cpu = sum(p["cpu_s"] - a.get(pid, {"cpu_s": 0})["cpu_s"] for pid, p in b.items())
    wake = sum(p["wakeups"] - a.get(pid, {"wakeups": 0})["wakeups"] for pid, p in b.items())
    return {"cpu_percent_of_one_core": round(100 * cpu / dt, 2), "wakeups_per_s": round(wake / dt, 1)}


# ------------------------------------------------------------------ local server
class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, speedometer=None):
        super().__init__(("127.0.0.1", 0), Handler)
        self.events, self.cond, self.speedometer, self.token = {}, threading.Condition(), speedometer, ""
        threading.Thread(target=self.serve_forever, daemon=True).start()

    def url(self, path):
        return f"http://127.0.0.1:{self.server_address[1]}{path}"

    def fire(self, token, value):
        with self.cond:
            self.events[token] = value
            self.cond.notify_all()

    def wait(self, token, timeout):
        with self.cond:
            if not self.cond.wait_for(lambda: token in self.events, timeout):
                raise TimeoutError(f"no callback for {token} within {timeout}s")
            return self.events[token]


SIMPLE = """<!doctype html><meta charset=utf-8><title>Benchmark running, please leave open</title>
<style>body{font:16px/1.5 system-ui;max-width:40em;margin:3em auto}</style>
<h1>Benchmark running, please leave this window open</h1>%s
<script>addEventListener('load',()=>fetch('/done?t=%s'))</script>"""
OPENER = """<!doctype html><title>Benchmark running, please leave open</title><script>
const sites=%s;let n=0;const timer=setInterval(()=>{
  if(n<sites.length-1)return void window.open(sites[n++]);
  clearInterval(timer);fetch('/done?t=%s').finally(()=>location.href=sites[n]);
},%d)</script>"""
REPORTER = """<script>(function poll(){
  const c=globalThis.benchmarkClient;if(!c)return setTimeout(poll,200);
  let hidden=document.hidden?1:0;addEventListener('visibilitychange',()=>{if(document.hidden)hidden++});
  const done=c.didFinishLastIteration.bind(c);
  c.didFinishLastIteration=m=>{done(m);fetch('/result?t=%s',{method:'POST',body:JSON.stringify({
    score:document.getElementById('result-number').textContent,
    interval:document.getElementById('confidence-number').textContent,
    iterations:c._measuredValuesList.map(v=>v.score),
    hidden_events:hidden,user_agent:navigator.userAgent})})};
})()</script>"""


class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def reply(self, body=b"", ctype="text/html", code=200):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def translate_path(self, path):  # static files: the Speedometer checkout under /sp/
        rel = urllib.parse.unquote(urllib.parse.urlsplit(path).path)[len("/sp/"):]
        root = pathlib.Path(self.server.speedometer).resolve()
        full = (root / rel).resolve()
        return str(full if full.is_relative_to(root) else root / "404")

    def do_GET(self):
        u = urllib.parse.urlsplit(self.path)
        token = urllib.parse.parse_qs(u.query).get("t", [""])[0]
        if u.path == "/simple":
            self.reply((SIMPLE % ("<p>The quick brown fox jumps over the lazy dog.</p>" * 20, token)).encode())
        elif u.path == "/opener":
            self.reply((OPENER % (json.dumps(SITES), token, OPEN_EVERY_MS)).encode())
        elif u.path == "/done":
            self.server.fire(token, time.monotonic())
            self.reply(code=204)
        elif u.path == "/sp/index.html" and self.server.speedometer:
            page = pathlib.Path(self.translate_path(u.path)).read_text()
            self.reply(page.replace("</body>", REPORTER % self.server.token + "</body>").encode())
        elif u.path.startswith("/sp/") and self.server.speedometer:
            super().do_GET()
        else:
            self.reply(code=404)

    def do_POST(self):
        token = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query).get("t", [""])[0]
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        self.server.fire(token, json.loads(body))
        self.reply(code=204)


# ------------------------------------------------------------------ launching
def launch(name, profile, urls, foreground=False):
    """Start a separate instance on `profile`; return its main process id."""
    exe = str(BROWSERS[name])
    app = exe.split("/Contents/MacOS/")[0]
    before = {pid for pid, _, comm in ps("comm") if comm == exe}
    profile.mkdir(parents=True, exist_ok=True)
    cmd = ["open", "-n"] + ([] if foreground else ["-g"])
    if name == "wisp":
        cfg = profile / "Library/Application Support/Wisp"
        cfg.mkdir(parents=True, exist_ok=True)
        tabs = [{"url": u, "title": "", "workspace": 1, "active": i == 0} for i, u in enumerate(urls)]
        (cfg / "session.json").write_text(json.dumps({"workspace": 1, "tabs": tabs}))
        # HOME moves Wisp's own files; CFFIXED_USER_HOME moves WebKit's data with them.
        cmd += ["--env", f"HOME={profile}", "--env", f"CFFIXED_USER_HOME={profile}", app]
    elif name == "zen":
        prefs = "".join(f"user_pref({json.dumps(k)}, {json.dumps(v)});\n" for k, v in ZEN_PREFS.items())
        (profile / "user.js").write_text(prefs)
        cmd += ["-a", app, "--args", "-profile", str(profile), "-no-remote", *urls]
    elif name == "chrome":
        cmd += ["-a", app, "--args", f"--user-data-dir={profile}", "--no-first-run",
                "--no-default-browser-check", *urls]  # fmt: skip
    subprocess.run(cmd, check=True)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        new = [pid for pid, _, comm in ps("comm") if comm == exe and pid not in before]
        if new:
            if name != "wisp":  # make sure it is ours, not one the owner just opened
                args = dict((pid, a) for pid, _, a in ps("args"))
                new = [pid for pid in new if str(profile) in args.get(pid, "")]
            if new:
                return new[0]
        time.sleep(0.02)
    raise RuntimeError(f"{name} did not start")


def alive(main):
    return any(pid == main for pid, _, _ in ps("comm"))


def quit(main):
    """Stop the instance we started, by process id only."""
    pids = tree(main)
    try:
        os.kill(main, signal.SIGTERM)
    except ProcessLookupError:  # it already went (someone closed the window): nothing to stop
        return
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline and any(pid == main for pid, _, _ in ps("comm")):
        time.sleep(0.25)
    time.sleep(1)
    now = dict((pid, comm) for pid, _, comm in ps("comm"))
    for pid, comm in pids.items():  # stragglers: same pid, same executable, still ours
        if now.get(pid) == comm and pid != 1:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    time.sleep(3)


def spread(values):
    return {"median": round(statistics.median(values), 2), "min": round(min(values), 2),
            "max": round(max(values), 2), "runs": [round(v, 2) for v in values]}  # fmt: skip


# ------------------------------------------------------------------ measurements
def bundle_size(app):
    kb = int(subprocess.run(["du", "-sk", os.path.realpath(app)], capture_output=True, text=True).stdout.split()[0])
    return round(kb / 1024, 1)


def cmd_size(args, server):
    out = {"wisp_binary_mb": round(BROWSERS["wisp"].stat().st_size / 2**20, 2)}
    apps = {"wisp": BROWSERS["wisp"].parents[2], "safari": "/Applications/Safari.app",
            "zen": "/Applications/Zen.app", "chrome": "/Applications/Google Chrome.app"}  # fmt: skip
    for name, app in apps.items():
        out[name] = {"bundle_mb": bundle_size(app)}
    # Wisp has no published macOS download yet: a zip of the app stands in for it.
    zipped = args.tmp / "Wisp.zip"
    args.tmp.mkdir(parents=True, exist_ok=True)
    subprocess.run(["ditto", "-c", "-k", "--keepParent", str(apps["wisp"]), str(zipped)], check=True)
    out["wisp"]["zip_mb"] = round(zipped.stat().st_size / 2**20, 2)
    return out


def cmd_coldstart(args, server):
    out = {}
    for name in args.browsers:
        profile, times = args.tmp / f"{name}-cold", []
        shutil.rmtree(profile, ignore_errors=True)
        for run in range(args.runs + 1):  # run 0 creates the profile and is reported apart
            token = f"{name}-cold-{run}"
            t0 = time.monotonic()
            main = launch(name, profile, [server.url(f"/simple?t={token}")], args.foreground)
            times.append(server.wait(token, 120) - t0)
            print(f"{name} cold start run {run}: {times[-1]:.3f}s", flush=True)
            time.sleep(2)
            quit(main)
        out[name] = {"first_run_fresh_profile_s": round(times[0], 3), "seconds": spread(times[1:])}
    return out


def cmd_onetab(args, server):
    """One simple page: memory SETTLE_10 seconds after launch, then a minute of idle CPU.
    Run -1 only does the browser's first-run work on the profile and is not recorded."""
    out = {}
    for name in args.browsers:
        runs, profile = [], args.tmp / f"{name}-one"
        shutil.rmtree(profile, ignore_errors=True)
        run = -1
        while run < args.runs:
            token = f"{name}-one-{run}-{time.monotonic_ns()}"
            t0 = time.monotonic()
            main = launch(name, profile, [server.url(f"/simple?t={token}")], args.foreground)
            server.wait(token, 120)
            time.sleep(max(0, (20 if run < 0 else SETTLE_10) - (time.monotonic() - t0)))
            if run >= 0:
                result = {**memory(main), **idle(main)}
                if not alive(main):  # closed during the measurement: the numbers are partial
                    print(f"{name} one tab run {run}: instance ended early, repeating the run", flush=True)
                    continue
                runs.append(result)
                print(f"{name} one tab run {run}: {runs[-1]['footprint_mb']} MB footprint, {runs[-1]['rss_mb']} MB rss, "
                      f"{runs[-1]['cpu_percent_of_one_core']}% cpu", flush=True)  # fmt: skip
            quit(main)
            run += 1
        out[name] = summarize(runs)
    return out


def summarize(runs):
    s = {k: spread([r[k] for r in runs]) for k in ("footprint_mb", "rss_mb", "processes")}
    for k in ("cpu_percent_of_one_core", "wakeups_per_s"):
        if k in runs[0]:
            s[k] = spread([r[k] for r in runs])
    s["by_kind_last_run"] = runs[-1]["by_kind"]
    return s


def cmd_tentabs(args, server):
    """Ten tabs: memory after SETTLE_10 seconds, then a minute of idle CPU."""
    out = {}
    modes = [(b, b) for b in args.browsers if b != "wisp"]
    if "wisp" in args.browsers:
        # Wisp builds a restored tab's web view only when the tab is first shown, so a
        # restored session is not ten loaded pages. "loaded" opens each site as its own
        # tab from a page (window.open), which Wisp shows, and so loads, one by one.
        modes += [("wisp_all_loaded", "wisp"), ("wisp_restored_not_shown", "wisp")]
    for run in range(args.runs):
        for label, name in modes:  # interleaved, so drift in the sites hits everyone alike
            while True:  # repeated if the instance ends before the measurement does
                profile, token = args.tmp / f"{label}-ten-{run}", f"{label}-ten-{run}-{time.monotonic_ns()}"
                shutil.rmtree(profile, ignore_errors=True)
                urls = [server.url(f"/opener?t={token}")] if label == "wisp_all_loaded" else SITES
                t0 = time.monotonic()
                main = launch(name, profile, urls, args.foreground)
                if label == "wisp_all_loaded":
                    try:
                        server.wait(token, 120)
                    except TimeoutError:  # the window went away before the opener finished
                        print(f"{label} ten tabs run {run}: opener never finished, repeating the run", flush=True)
                        quit(main)
                        continue
                time.sleep(max(0, SETTLE_10 - (time.monotonic() - t0)))
                result = memory(main)
                result.update(idle(main))
                if not alive(main):  # closed during the measurement: the numbers are partial
                    print(f"{label} ten tabs run {run}: instance ended early, repeating the run", flush=True)
                    continue
                print(f"{label} ten tabs run {run}: {result['footprint_mb']} MB footprint, {result['rss_mb']} MB rss, "
                      f"{result['processes']} processes, {result['cpu_percent_of_one_core']}% cpu", flush=True)  # fmt: skip
                out.setdefault(label, []).append(result)
                quit(main)
                break
    return {label: summarize(runs) for label, runs in out.items()}


def cmd_speedometer(args, server):
    out = {}
    for name in args.browsers:
        runs = []
        for run in range(args.runs):
            profile = args.tmp / f"{name}-speedometer"
            server.token = f"{name}-sp-{run}"
            main = launch(name, profile, [server.url("/sp/index.html?startAutomatically=true")], args.foreground)
            try:
                runs.append(server.wait(server.token, 900))
                print(f"{name} Speedometer run {run}: {runs[-1]['score']} {runs[-1]['interval']} "
                      f"(hidden events: {runs[-1]['hidden_events']})", flush=True)  # fmt: skip
            finally:
                quit(main)
        out[name] = {"score": spread([float(r["score"]) for r in runs]), "runs_detail": runs}
    return out


def machine():
    sysctl = lambda k: subprocess.run(["sysctl", "-n", k], capture_output=True, text=True).stdout.strip()
    version = lambda app: subprocess.run(
        ["defaults", "read", f"{app}/Contents/Info", "CFBundleShortVersionString"], capture_output=True, text=True
    ).stdout.strip()
    return {
        "model": sysctl("hw.model"), "chip": sysctl("machdep.cpu.brand_string"), "cores": int(sysctl("hw.ncpu")),
        "memory_gb": int(sysctl("hw.memsize")) // 2**30, "macos": platform.mac_ver()[0],
        "versions": {
            "wisp": subprocess.run(["git", "-C", str(REPO), "describe", "--always", "--dirty"], capture_output=True, text=True).stdout.strip(),
            "safari": version("/Applications/Safari.app"), "zen": version("/Applications/Zen.app"),
            "chrome": version("/Applications/Google Chrome.app"),
        },
    }  # fmt: skip


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("what", choices=["size", "coldstart", "onetab", "tentabs", "speedometer"])
    ap.add_argument("--browsers", default="wisp,zen,chrome", type=lambda s: s.split(","))
    ap.add_argument("--runs", type=int)
    ap.add_argument("--foreground", action="store_true")
    ap.add_argument("--src", help="Speedometer checkout (speedometer only)")
    ap.add_argument("--tmp", type=pathlib.Path, default=pathlib.Path("/tmp/wisp-bench"))
    ap.add_argument("--out", type=pathlib.Path, default=REPO / "docs/performance-data.json")
    ap.add_argument("--wisp-app", type=pathlib.Path)
    args = ap.parse_args()
    if args.wisp_app:
        BROWSERS["wisp"] = args.wisp_app.resolve() / "Contents/MacOS/wisp"
    args.runs = args.runs or (5 if args.what == "coldstart" else 3)
    if args.what == "speedometer" and not args.src:
        ap.error("speedometer needs --src")
    server = Server(args.src)
    result = globals()[f"cmd_{args.what}"](args, server)
    data = json.loads(args.out.read_text()) if args.out.exists() else {}
    data["machine"] = machine()
    data[args.what] = {"measured_at": time.strftime("%Y-%m-%d %H:%M"), **result}
    args.out.write_text(json.dumps(data, indent=1) + "\n")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
