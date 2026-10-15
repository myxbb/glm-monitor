#!/usr/bin/env python3
"""生成 Tauri 应用图标（src-tauri/icons/）。

用法：
    python3 scripts/gen_icons.py [master.png]

- 提供(master.png)时：居中裁剪白边 → 按 macOS 规范缩放到 824x824 放入
  1024 透明画布 → 圆角遮罩（Big Sur 模板：圆角半径 185）→ 生成全套图标
- 未提供时：退回内置的纯代码渐变图标

输出（icns/ico/PNG）仅用 macOS 自带 sips/iconutil + ico 以 PNG 负载拼装。
重复执行会覆盖旧图标。
"""
import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICONS = os.path.join(ROOT, "src-tauri", "icons")

GREEN = (52, 199, 89)     # #34C759
GREEN_D = (36, 168, 70)   # 渐变底部
WHITE = (255, 255, 255)

MASTER = 1024
CANVAS = 1024
ART = 824                 # macOS 图标模板中图案区域的边长
RADIUS = 185              # 模板圆角半径


def rounded_square(size):
    """内置兜底：渐变圆角方形 + 中心白色圆点，返回 RGBA 字节。"""
    buf = bytearray()
    half = size / 2.0
    dot_r = size * 0.30
    r0 = ART / 2 - RADIUS
    for y in range(size):
        for x in range(size):
            # 垂直渐变
            t = y / (size - 1)
            r = int(GREEN[0] + (GREEN_D[0] - GREEN[0]) * t)
            g = int(GREEN[1] + (GREEN_D[1] - GREEN[1]) * t)
            b = int(GREEN[2] + (GREEN_D[2] - GREEN[2]) * t)
            # 圆角 SDF（图案区 824 居中）
            cx, cy = x - (size - ART) / 2.0, y - (size - ART) / 2.0
            ax = min(max(cx, r0), ART - r0)
            ay = min(max(cy, r0), ART - r0)
            d = math.hypot(cx - ax, cy - ay)
            alpha = 1.0 if d <= r0 - 1 else max(0.0, min(1.0, r0 - d))
            cd = math.hypot(x - half + 0.5, y - half + 0.5)
            if cd <= dot_r - 1:
                r, g, b = WHITE
            elif cd <= dot_r:
                k = dot_r - cd
                r = int(r + (WHITE[0] - r) * k)
                g = int(g + (WHITE[1] - g) * k)
                b = int(b + (WHITE[2] - b) * k)
            buf.extend([r, g, b, int(alpha * 255)])
    return bytes(buf)


def master_from_source(src):
    """把用户提供的源图转成 1024 RGBA 主图（去白边、缩放、圆角透明遮罩）。"""
    from PIL import Image, ImageDraw, ImageOps

    im = Image.open(src).convert("RGBA")
    # 裁掉四周近白边缘与原图自带投影光晕：
    # 阈值收紧 + 向内多收 3%，确保圆角矩形主体铺满画布图案区
    gray = ImageOps.grayscale(im)
    mask_bw = gray.point(lambda p: 255 if p < 238 else 0)
    bbox = mask_bw.getbbox()
    if bbox:
        pad = int(max(im.size) * 0.03)
        bbox = (min(bbox[0] + pad, im.width), min(bbox[1] + pad, im.height),
                max(bbox[2] - pad, 0), max(bbox[3] - pad, 0))
        if bbox[2] > bbox[0] and bbox[3] > bbox[1]:
            im = im.crop(bbox)
    # 短边对齐后居中裁方形
    side = min(im.size)
    im = ImageOps.fit(im, (side, side), Image.LANCZOS)
    im = im.resize((ART, ART), Image.LANCZOS)

    canvas = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    # 圆角遮罩（超采样抗锯齿）
    m = Image.new("L", (ART * 4, ART * 4), 0)
    ImageDraw.Draw(m).rounded_rectangle([0, 0, ART * 4 - 1, ART * 4 - 1],
                                        radius=RADIUS * 4, fill=255)
    m = m.resize((ART, ART), Image.LANCZOS)
    canvas.paste(im, ((CANVAS - ART) // 2, (CANVAS - ART) // 2), m)
    return canvas


def write_png(path, size, rgba):
    def chunk(typ, data):
        c = struct.pack(">I", len(data)) + typ + data
        return c + struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    raw = b"".join(b"\x00" + rgba[y * size * 4:(y + 1) * size * 4] for y in range(size))
    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", ihdr)
           + chunk(b"IDAT", zlib.compress(raw, 9))
           + chunk(b"IEND", b""))
    with open(path, "wb") as f:
        f.write(png)


def resize(src, dst, size):
    subprocess.run(["sips", "-z", str(size), str(size), src, "--out", dst],
                   check=True, capture_output=True)


def make_icns(master_png):
    iconset = tempfile.mkdtemp(suffix=".iconset")
    sizes = [16, 32, 128, 256, 512]
    for s in sizes:
        resize(master_png, os.path.join(iconset, f"icon_{s}x{s}.png"), s)
        resize(master_png, os.path.join(iconset, f"icon_{s}x{s}@2x.png"), s * 2)
    subprocess.run(["iconutil", "-c", "icns", iconset, "-o",
                    os.path.join(ICONS, "icon.icns")], check=True)
    shutil.rmtree(iconset)


def make_ico(master_png):
    entries = []
    data = b""
    offset = 6 + 16 * 4
    for s in (16, 32, 48, 64, 128, 256):
        tmp = tempfile.NamedTemporaryFile(suffix=".png", delete=False)
        tmp.close()
        resize(master_png, tmp.name, s)
        with open(tmp.name, "rb") as f:
            png = f.read()
        os.unlink(tmp.name)
        w = 0 if s >= 256 else s
        entries.append(struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(png), offset))
        data += png
        offset += len(png)
    with open(os.path.join(ICONS, "icon.ico"), "wb") as f:
        f.write(struct.pack("<HHH", 0, 1, len(entries)) + b"".join(entries) + data)


def main():
    os.makedirs(ICONS, exist_ok=True)
    master = os.path.join(ICONS, "icon.png")
    if len(sys.argv) > 1:
        img = master_from_source(sys.argv[1])
        img.save(master)
    else:
        write_png(master, MASTER, rounded_square(MASTER))

    for s in (32, 128):
        resize(master, os.path.join(ICONS, f"{s}x{s}.png"), s)
    resize(master, os.path.join(ICONS, "128x128@2x.png"), 256)
    make_icns(master)
    make_ico(master)
    print("icons written to", ICONS)


if __name__ == "__main__":
    sys.exit(main())
