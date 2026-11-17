"""Write latest.json, the file installed copies of Wisp read to find updates.

usage: latest-json.py <dir with release files> <tag, e.g. v0.2.0> <owner/repo> [base url]

Every update file must sit next to its `.sig` (made by the release workflow with the
update signing key). Without any signed file nothing is written: a release without
latest.json simply offers no automatic update.
"""
import json
import os
import sys

directory, tag, repo = sys.argv[1:4]
base = sys.argv[4] if len(sys.argv) > 4 else f"https://github.com/{repo}/releases/download/{tag}/"

# what the updater calls each platform, how its update file's name ends, and its kind
RULES = [
    ("macos-aarch64", "-macOS-Apple-Silicon.update.tar.gz", "app"),
    ("macos-x86_64", "-macOS-Intel.update.tar.gz", "app"),
]
# Windows automatic updates are off unless WISP_WINDOWS_UPDATES is "on" (a repository
# variable the release workflow passes in). Defender quarantined Wisp when 0.2.0
# updated itself to 0.2.1; see docs/WINDOWS-SECURITY.md for what to check on a real
# Windows PC before turning this on. Off, installed Windows copies find no update.
if os.environ.get("WISP_WINDOWS_UPDATES") == "on":
    RULES.append(("windows-x86_64", "-Windows-x64-Setup.exe", "nsis"))
else:
    print("Windows automatic updates are off (WISP_WINDOWS_UPDATES is not \"on\")")

platforms = {}
for target, ending, kind in RULES:
    for name in sorted(os.listdir(directory)):
        signature = os.path.join(directory, name + ".sig")
        if name.endswith(ending) and os.path.exists(signature):
            platforms[target] = {"url": base + name, "signature": open(signature).read().strip(), "format": kind}

if not platforms:
    print("no signed update files: latest.json not written")
    sys.exit(0)

path = os.path.join(directory, "latest.json")
with open(path, "w") as f:
    json.dump({"version": tag.removeprefix("v"), "platforms": platforms}, f, indent=2)
print(f"wrote {path} for {', '.join(platforms)}")
