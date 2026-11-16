#!/usr/bin/env python3
"""Memory of Wisp's interface web view, by build and by appearance setting.

    python3 bench/interface_memory.py LABEL=path/to/Wisp.app[:config.json] ... [--runs 3]

Each run: one simple tab, sampled 40 s after it loads. The interface's web-content
process is the first one Wisp starts (the interface is built before any tab), so it is
the lowest-numbered com.apple.WebKit.WebContent in Wisp's tree. Its memory is broken
down with macOS's `footprint` tool; "graphics" is GPU-side memory (layer backing
stores, IOSurfaces) the process owns.
"""
import json, pathlib, re, shutil, statistics, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import bench

UNIT = {"B": 1 / 2**20, "KB": 1 / 1024, "MB": 1, "GB": 1024}


def categories(pid):
    out = subprocess.run(["footprint", "-p", str(pid)], capture_output=True, text=True).stdout
    cats = {}
    for line in out.splitlines():
        m = re.match(r"\s*([\d.]+) (B|KB|MB|GB)\s+\S+ \S+\s+\S+ \S+\s+\d+\s+(.+)", line)
        if m:
            cats[m[3].strip()] = round(float(m[1]) * UNIT[m[2]], 1)
    return cats


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    runs = int(sys.argv[sys.argv.index("--runs") + 1]) if "--runs" in sys.argv else 3
    args = [a for a in args if not a.isdigit()]
    server, tmp, results = bench.Server(), pathlib.Path("/tmp/wisp-bench-ui"), {}
    for run in range(runs):
        for spec in args:  # interleaved so drift hits every variant alike
            label, rest = spec.split("=", 1)
            app, _, cfg = rest.partition(":")
            bench.BROWSERS["wisp"] = pathlib.Path(app).resolve() / "Contents/MacOS/wisp"
            profile = tmp / f"{label}-{run}"
            shutil.rmtree(profile, ignore_errors=True)
            if cfg:
                d = profile / "Library/Application Support/Wisp"
                d.mkdir(parents=True)
                shutil.copy(cfg, d / "config.json")
            for attempt in range(5):  # repeated if the instance goes away mid-measurement
                token = f"{label}-{run}-{attempt}"
                main_pid = bench.launch("wisp", profile, [server.url(f"/simple?t={token}")])
                try:
                    server.wait(token, 60)
                    time.sleep(40)
                    procs = bench.sample(main_pid)
                    ui = min(p for p, q in procs.items() if q["name"] == "com.apple.WebKit.WebContent")
                    cats = categories(ui)
                    if bench.alive(main_pid):
                        break
                except (TimeoutError, ValueError):
                    pass
                print(label, run, "instance ended early, repeating", flush=True)
                bench.quit(main_pid)
            r = {"tree_mb": bench.mb(sum(q["footprint"] for q in procs.values())),
                 "interface_mb": bench.mb(procs[ui]["footprint"]),
                 "graphics_mb": cats.get("Owned physical footprint (unmapped) (graphics)", 0.0),
                 "webkit_malloc_mb": cats.get("WebKit malloc", 0.0),
                 "top": dict(sorted(cats.items(), key=lambda kv: -kv[1])[:6])}  # fmt: skip
            print(label, run, json.dumps(r), flush=True)
            results.setdefault(label, []).append(r)
            bench.quit(main_pid)
    summary = {
        label: {k: statistics.median(r[k] for r in rs) for k in ("tree_mb", "interface_mb", "graphics_mb", "webkit_malloc_mb")}
        | {"runs": rs}
        for label, rs in results.items()
    }  # fmt: skip
    print(json.dumps({k: {m: v for m, v in s.items() if m != "runs"} for k, s in summary.items()}, indent=1))
    pathlib.Path(tmp / "result.json").write_text(json.dumps(summary, indent=1))


if __name__ == "__main__":
    main()
