"""Generate BDF's own generic app icons (contributor-original, no fonts, no third-party artwork):
a rounded navy tile with the letters B D F built from round-capped strokes, and a PDF document
icon. Pure standard library: signed-distance rendering with analytic anti-aliasing.

Usage: python icons.py OUTDIR    writes bdf.svg bdf-1024.png bdf.ico bdf.icns bdf-document.svg
                                 bdf-document.ico and hicolor/<n>x<n>/apps/<id>.png
"""
import math, os, struct, sys, zlib

NAVY = (0x1F, 0x3A, 0x5F)
WHITE = (0xFF, 0xFF, 0xFF)
FOLD = (0xC9, 0xD3, 0xE0)
ID = 'ai.storyteller.pdfcraft'  # the Linux icon theme name stays the packaged application id


# ---- geometry: stroked paths made of lines and circular arcs -------------------------------------
class Path:
    def __init__(self):
        self.subs = []

    def move(self, x, y):
        self.subs.append([('M', x, y)])
        return self

    def line(self, x, y):
        self.subs[-1].append(('L', x, y))
        return self

    def arc(self, cx, cy, r, a0, a1):
        """Clockwise (screen) arc from angle a0 to a1 degrees about (cx, cy)."""
        self.subs[-1].append(('A', cx, cy, r, a0, a1))
        return self

    def svg(self):
        out = []
        for sub in self.subs:
            for op in sub:
                if op[0] == 'M':
                    out.append(f'M{op[1]:g} {op[2]:g}')
                elif op[0] == 'L':
                    out.append(f'L{op[1]:g} {op[2]:g}')
                else:
                    _, cx, cy, r, a0, a1 = op
                    ex, ey = cx + r * math.cos(math.radians(a1)), cy + r * math.sin(math.radians(a1))
                    large = 1 if (a1 - a0) % 360 > 180 else 0
                    out.append(f'A{r:g} {r:g} 0 {large} 1 {ex:.3f} {ey:.3f}')
        return ' '.join(out)

    def segments(self):
        segs = []
        for sub in self.subs:
            cur = None
            for op in sub:
                if op[0] == 'M':
                    cur = (op[1], op[2])
                elif op[0] == 'L':
                    segs.append((cur[0], cur[1], op[1], op[2]))
                    cur = (op[1], op[2])
                else:
                    _, cx, cy, r, a0, a1 = op
                    n = max(8, int(abs(a1 - a0) / 4))
                    prev = (cx + r * math.cos(math.radians(a0)), cy + r * math.sin(math.radians(a0)))
                    for i in range(1, n + 1):
                        a = math.radians(a0 + (a1 - a0) * i / n)
                        nxt = (cx + r * math.cos(a), cy + r * math.sin(a))
                        segs.append((prev[0], prev[1], nxt[0], nxt[1]))
                        prev = nxt
                    cur = prev
        return segs


def letters_bdf(dx=-15.0):
    p = Path()
    # B
    x = 90 + dx
    p.move(x, 180).line(x, 332)
    p.move(x, 180).line(x + 38, 180).arc(x + 38, 218, 38, -90, 90).line(x, 256)
    p.move(x, 256).line(x + 38, 256).arc(x + 38, 294, 38, -90, 90).line(x, 332)
    # D
    x = 226 + dx
    p.move(x, 180).line(x, 332)
    p.move(x, 180).line(x + 24, 180).arc(x + 24, 256, 76, -90, 90).line(x, 332)
    # F
    x = 386 + dx
    p.move(x, 180).line(x, 332)
    p.move(x, 180).line(x + 66, 180)
    p.move(x, 250).line(x + 50, 250)
    return p


def letters_pdf():
    p = Path()
    yt, yb = 308.0, 376.0
    x = 150.0  # P
    p.move(x, yt).line(x, yb)
    p.move(x, yt).line(x + 22, yt).arc(x + 22, yt + 20, 20, -90, 90).line(x, yt + 40)
    x = 232.0  # D
    p.move(x, yt).line(x, yb)
    p.move(x, yt).line(x + 10, yt).arc(x + 10, (yt + yb) / 2, 34, -90, 90).line(x, yb)
    x = 318.0  # F
    p.move(x, yt).line(x, yb)
    p.move(x, yt).line(x + 50, yt)
    p.move(x, yt + 30).line(x + 38, yt + 30)
    return p


# ---- signed distances ---------------------------------------------------------------------------
def seg_dist(px, py, s):
    ax, ay, bx, by = s
    vx, vy = bx - ax, by - ay
    wx, wy = px - ax, py - ay
    l2 = vx * vx + vy * vy
    t = 0.0 if l2 == 0 else max(0.0, min(1.0, (wx * vx + wy * vy) / l2))
    dx, dy = wx - t * vx, wy - t * vy
    return math.hypot(dx, dy)


def rrect_sd(px, py, x0, y0, x1, y1, r):
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    hx, hy = (x1 - x0) / 2 - r, (y1 - y0) / 2 - r
    qx, qy = abs(px - cx) - hx, abs(py - cy) - hy
    return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - r


def cover(sd, scale):
    """Coverage from a signed distance in 512-space (negative inside), at `scale` px per unit."""
    return max(0.0, min(1.0, 0.5 - sd * scale))


def over(dst, src, a):
    """src colour at coverage a over dst = (r, g, b, alpha)."""
    r, g, b, da = dst
    oa = a + da * (1 - a)
    if oa <= 0:
        return (0.0, 0.0, 0.0, 0.0)
    f = lambda s, d: (s * a + d * da * (1 - a)) / oa
    return (f(src[0], r), f(src[1], g), f(src[2], b), oa)


# ---- rendering ------------------------------------------------------------------------------------
def render_app(size, tile_scale=1.0, tile_offset=0.0):
    """The app icon at size x size px. The tile is 512 units wide, drawn at tile_scale of the canvas."""
    segs = letters_bdf().segments()
    sc = size / 512.0 * tile_scale  # px per tile unit
    off = tile_offset / 512.0 * size
    stroke_r = 20.0
    rows = []
    for j in range(size):
        row = []
        for i in range(size):
            px, py = ((i + 0.5) - off) / sc, ((j + 0.5) - off) / sc
            a_tile = cover(rrect_sd(px, py, 0, 0, 512, 512, 112), sc)
            if a_tile <= 0:
                row.append((0.0, 0.0, 0.0, 0.0))
                continue
            pix = (NAVY[0], NAVY[1], NAVY[2], a_tile)
            if 40 < px < 470 and 150 < py < 365:
                d = min(seg_dist(px, py, s) for s in segs) - stroke_r
                a = cover(d, sc)
                if a > 0:
                    pix = over(pix, WHITE, a)
            row.append(pix)
        rows.append(row)
    return rows


def render_document(size):
    segs = letters_pdf().segments()
    sc = size / 512.0
    rows = []
    for j in range(size):
        row = []
        for i in range(size):
            px, py = (i + 0.5) / sc, (j + 0.5) / sc
            pix = (0.0, 0.0, 0.0, 0.0)
            # page: navy outline, white sheet, cut corner with a folded flap
            outer = rrect_sd(px, py, 88, 40, 424, 472, 36)
            cut = ((px - 340) - (py - 48)) / math.sqrt(2)  # > 0 beyond the diagonal of the top-right corner
            a_out = cover(max(outer, cut), sc)
            if a_out > 0:
                pix = over(pix, NAVY, a_out)
                inner = rrect_sd(px, py, 100, 52, 412, 460, 26)
                a_in = cover(max(inner, cut + 8 * math.sqrt(2) / 2), sc)
                if a_in > 0:
                    pix = over(pix, WHITE, a_in)
            # the folded flap: the triangle (340,48) (340,124) (416,124)
            in_tri = (px >= 340 and py <= 124 and (px - 340) <= (py - 48))
            if in_tri and a_out > 0:
                edge = min(px - 340, 124 - py, ((py - 48) - (px - 340)) / math.sqrt(2))
                pix = over(pix, FOLD, cover(-edge, sc))
            # the banner carrying the lettering, across the lower sheet
            if 292 <= py <= 392 or abs(py - 342) < 60:
                bsd = max(py - 392, 292 - py, 96 - px, px - 416)
                a_b = cover(bsd, sc)
                if a_b > 0 and a_out > 0:
                    pix = over(pix, NAVY, a_b)
            if 280 < py < 400 and 120 < px < 400:
                d = min(seg_dist(px, py, s) for s in segs) - 10.0
                a = cover(d, sc)
                if a > 0:
                    pix = over(pix, WHITE, a)
            row.append(pix)
        rows.append(row)
    return rows


# ---- encoders ---------------------------------------------------------------------------------------
def png(rows):
    h, w = len(rows), len(rows[0])
    raw = bytearray()
    for row in rows:
        raw.append(0)
        for r, g, b, a in row:
            raw += bytes((round(r), round(g), round(b), round(a * 255)))
    def chunk(tag, data):
        c = struct.pack('>I', len(data)) + tag + data
        return c + struct.pack('>I', zlib.crc32(tag + data) & 0xFFFFFFFF)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(bytes(raw), 9)) + chunk(b'IEND', b'')


def ico(pngs):
    """pngs: list of (size, png bytes)."""
    head = struct.pack('<HHH', 0, 1, len(pngs))
    entries, blobs, offset = b'', b'', 6 + 16 * len(pngs)
    for size, data in pngs:
        entries += struct.pack('<BBBBHHII', 0 if size >= 256 else size, 0 if size >= 256 else size, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    return head + entries + blobs


def icns(pngs):
    """pngs: dict type -> png bytes."""
    body = b''.join(t + struct.pack('>I', 8 + len(d)) + d for t, d in pngs.items())
    return b'icns' + struct.pack('>I', 8 + len(body)) + body


def app_svg():
    d = letters_bdf().svg()
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">\n'
            '  <!-- BDF app icon: contributor-original; letters are stroked paths, no font. MIT OR Apache-2.0. -->\n'
            '  <rect width="512" height="512" rx="112" fill="#1f3a5f"/>\n'
            f'  <path d="{d}" fill="none" stroke="#ffffff" stroke-width="40" stroke-linecap="round" stroke-linejoin="round"/>\n'
            '</svg>\n')


def document_svg():
    d = letters_pdf().svg()
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">\n'
            '  <!-- BDF PDF document icon: contributor-original; lettering is stroked paths, no font. MIT OR Apache-2.0. -->\n'
            '  <path d="M124 40H340L424 124V436Q424 472 388 472H124Q88 472 88 436V76Q88 40 124 40Z" fill="#1f3a5f"/>\n'
            '  <path d="M126 52H332L412 132V434Q412 460 386 460H126Q100 460 100 434V78Q100 52 126 52Z" fill="#ffffff"/>\n'
            '  <path d="M340 48V124H416Z" fill="#c9d3e0"/>\n'
            '  <rect x="96" y="292" width="320" height="100" fill="#1f3a5f"/>\n'
            f'  <path d="{d}" fill="none" stroke="#ffffff" stroke-width="20" stroke-linecap="round" stroke-linejoin="round"/>\n'
            '</svg>\n')


def main(out):
    os.makedirs(out, exist_ok=True)
    open(os.path.join(out, 'bdf.svg'), 'w', newline='\n').write(app_svg())
    open(os.path.join(out, 'bdf-document.svg'), 'w', newline='\n').write(document_svg())
    cache = {}
    def app_png(n):
        if n not in cache:
            cache[n] = png(render_app(n))
        return cache[n]
    # Linux icon theme sizes (full-bleed tile)
    for n in (16, 24, 32, 48, 64, 128, 256, 512):
        d = os.path.join(out, 'hicolor', f'{n}x{n}', 'apps')
        os.makedirs(d, exist_ok=True)
        open(os.path.join(d, ID + '.png'), 'wb').write(app_png(n))
    os.makedirs(os.path.join(out, 'hicolor', 'scalable', 'apps'), exist_ok=True)
    open(os.path.join(out, 'hicolor', 'scalable', 'apps', ID + '.svg'), 'w', newline='\n').write(app_svg())
    # Windows .ico
    sizes = (16, 20, 24, 32, 40, 48, 64, 128, 256)
    open(os.path.join(out, 'bdf.ico'), 'wb').write(ico([(n, app_png(n)) for n in sizes]))
    open(os.path.join(out, 'bdf-document.ico'), 'wb').write(ico([(n, png(render_document(n))) for n in sizes]))
    # macOS: the tile on Apple's grid (an 824 px body centred on a transparent 1024 px canvas)
    mac1024 = png(render_app(1024, tile_scale=824 / 1024, tile_offset=100 / 1024 * 512))
    open(os.path.join(out, 'bdf-1024.png'), 'wb').write(mac1024)
    def mac(n):
        return png(render_app(n, tile_scale=824 / 1024, tile_offset=100 / 1024 * 512))
    open(os.path.join(out, 'bdf.icns'), 'wb').write(icns({b'ic07': mac(128), b'ic08': mac(256), b'ic09': mac(512), b'ic10': mac1024}))
    print('wrote', out)


if __name__ == '__main__':
    main(sys.argv[1])
