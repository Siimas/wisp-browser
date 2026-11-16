"""Regenerates icon.png / icon.ico / icon.icns (macOS: needs sips + iconutil). Run: python3 make_icon.py"""
import math, os, struct, subprocess, zlib

N = 1024

def cover(d):  # signed distance -> 0..1 coverage (1px anti-aliasing)
    return min(1.0, max(0.0, 0.5 - d))

def pixel(x, y):
    # rounded-square background with a diagonal gradient
    m, r = 100, 230
    qx, qy = abs(x - N / 2) - (N / 2 - m - r), abs(y - N / 2) - (N / 2 - m - r)
    bg = cover(math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - r)
    t = (x + y) / (2 * N)
    col = [0x6c + (0xa8 - 0x6c) * t, 0x8c + (0x6c - 0x8c) * t, 0xff]
    # white crescent: a disc minus an offset disc
    disc = cover(math.hypot(x - 512, y - 512) - 270)
    cut = cover(math.hypot(x - 610, y - 430) - 230)
    w = disc * (1 - cut)
    return bytes(int(c + (255 - c) * w) for c in col) + bytes([int(255 * bg)])

def png(path):
    raw = b"".join(b"\0" + b"".join(pixel(x + .5, y + .5) for x in range(N)) for y in range(N))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", N, N, 8, 6, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))

png("icon.png")
os.makedirs("icon.iconset", exist_ok=True)
for s in (16, 32, 128, 256, 512):
    for scale in (1, 2):
        name = f"icon.iconset/icon_{s}x{s}{'@2x' if scale == 2 else ''}.png"
        subprocess.run(["sips", "-z", str(s * scale), str(s * scale), "icon.png", "--out", name], check=True, capture_output=True)
subprocess.run(["iconutil", "-c", "icns", "icon.iconset", "-o", "icon.icns"], check=True)
data = open("icon.iconset/icon_256x256.png", "rb").read()  # .ico = header + one embedded 256px PNG
with open("icon.ico", "wb") as f:
    f.write(struct.pack("<HHH", 0, 1, 1) + struct.pack("<BBBBHHII", 0, 0, 0, 0, 1, 32, len(data), 22) + data)
subprocess.run(["rm", "-r", "icon.iconset"])
