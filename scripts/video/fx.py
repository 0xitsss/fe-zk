"""Tiny scene toolkit for the fe-zk demo video: ffmpeg filter-graph builder (stdlib only).

A Scene is one background still plus a chain of drawtext/drawbox filters (fade/slide-in via
time expressions) and optional extra video inputs (Life boards). Scenes render to intermediate
mp4 files and are joined with xfade transitions by `assemble`.
Everything shown is passed in by build.py from real command output / repository files.
"""
import math, os, random, re, subprocess, sys
from pathlib import Path

W, H, FPS = 1920, 1080, 30
LIB = "/usr/share/fonts/truetype/liberation/"
DJV = "/usr/share/fonts/truetype/dejavu/"
SANS, SANSB = LIB + "LiberationSans-Regular.ttf", LIB + "LiberationSans-Bold.ttf"
MONO, MONOB = DJV + "DejaVuSansMono.ttf", DJV + "DejaVuSansMono-Bold.ttf"
SYM = DJV + "DejaVuSans.ttf"
CHARW = 1233 / 2048.0  # DejaVu Sans Mono advance, in em

# palette (one accent: sky blue; orange only for the window minimize dot)
ACC = "0x4fc3f7"
ACC2 = "0xa6dcff"
BG0 = "0x0b0f14"
PANEL = "0x0d1117"
PANEL2 = "0x161b22"
LINE = "0x30363d"
TXT = "0xe6edf3"
DIM = "0x8b949e"
FAINT = "0x6e7681"
OK = "0x3fb950"
ERR = "0xf85149"
BLUE = "0x79c0ff"
PURPLE = "0xd2a8ff"
STR = "0xa5d6ff"
PINK = "0xd2a8ff"

ENC = ["-c:v", "libx264", "-preset", "veryfast", "-crf", "15", "-pix_fmt", "yuv420p", "-r", str(FPS), "-an"]


def sh(cmd, **kw):
    p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, **kw)
    return p.returncode, p.stdout


def cw(size):
    return size * CHARW


def make_background(path, tmp):
    """Dark vertical gradient, soft accent glow top-left, faint dot grid, dither noise."""
    rnd = random.Random(7)
    rows = []
    for y in range(H):
        row = bytearray()
        for x in range(0, W):
            v = 0.0
            # base gradient
            r, g, b = 11 + 5 * (1 - y / H), 15 + 6 * (1 - y / H), 20 + 8 * (1 - y / H)
            # glow
            d = math.hypot(x - 180, y - 120) / 900.0
            k = max(0.0, 1 - d) ** 2 * 0.10
            r += 255 * k * 0.55
            g += 122 * k * 0.55
            b += 26 * k * 0.55
            # vignette
            vx, vy = (x - W / 2) / (W / 2), (y - H / 2) / (H / 2)
            vg = 1 - 0.28 * (vx * vx + vy * vy) / 2
            r, g, b = r * vg, g * vg, b * vg
            if x % 64 == 0 and y % 64 == 0:
                r, g, b = r + 14, g + 16, b + 20
            n = rnd.random() - 0.5
            row += bytes((max(0, min(255, int(r + n))), max(0, min(255, int(g + n))), max(0, min(255, int(b + n)))))
        rows.append(bytes(row))
    ppm = Path(tmp) / "bg.ppm"
    ppm.write_bytes(b"P6\n%d %d\n255\n" % (W, H) + b"".join(rows))
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", str(ppm), str(path)], check=True)
    ppm.unlink()


# ------------------------------------------------------------------------------------ syntax
KW = set("""pub fn const struct impl use let var mut while if else uses msg contract recv init return static_assert
match as for in mod trait enum where unsafe true false ingot super self loop break continue type extern
""".split())
BUILTIN = set("u8 u16 u32 u64 u128 u256 usize i32 i64 i256 bool Self".split())
TOK = re.compile(r"""(?P<com>//.*)|(?P<str>"[^"]*")|(?P<attr>\#\[[^\]]*\])|(?P<num>0x[0-9a-fA-F]+|\b\d[\d_]*\b)|(?P<id>[A-Za-z_][A-Za-z_0-9]*)|(?P<sp>\s+)|(?P<op>.)""")


def tokenize(line):
    """[(col, text, colour)] for one line of Fe source / shell, merged by colour."""
    out = []
    for m in TOK.finditer(line):
        k, s = m.lastgroup, m.group()
        if k == "com":
            c = FAINT
        elif k == "str":
            c = STR
        elif k == "attr":
            c = PURPLE
        elif k == "num":
            c = BLUE
        elif k == "id":
            if s in KW:
                c = PINK
            elif s in BUILTIN or s[0].isupper():
                c = PURPLE if not s.isupper() or len(s) < 3 else BLUE
            elif line[m.end():m.end() + 1] == "(" or line[m.end():m.end() + 1] == "<":
                c = ACC2
            else:
                c = TXT
        elif k == "sp":
            c = None
        else:
            c = DIM
        out.append((m.start(), s, c))
    merged = []
    for col, s, c in out:
        if c is None:
            if merged:
                merged[-1][1] += s
            continue
        if merged and merged[-1][2] == c and merged[-1][0] + len(merged[-1][1]) == col:
            merged[-1][1] += s
        else:
            merged.append([col, s, c])
    return [(a, b.rstrip(), c) for a, b, c in merged if b.strip()]


def term_colour(line):
    s = line.strip()
    if s.startswith("$ "):
        return TXT
    if re.search(r"error|FAILED|FAIL |REJECTED|static assertion failed|not visible|\^\^\^", line):
        return ERR
    if re.search(r"\bPASS\b|test result: ok|VALID", line):
        return OK
    if s.startswith(("#", "//")):
        return FAINT
    return "0xc9d1d9"


class Scene:
    def __init__(self, name, dur, tmp, title=None, chapter=None, idx=None, nchap=9):
        self.name, self.dur, self.tmp = name, float(dur), Path(tmp)
        self.chain, self.fc, self.inputs, self.nin, self.cur, self.k = [], [], [], 1, "bg", 0
        self.first_t0 = None
        self.last_t0 = 0.0
        self.rows = set()
        self.n = 0
        if title:
            self.chrome(title, chapter, idx, nchap)

    # ---- graph plumbing
    def f(self, flt):
        self.chain.append(flt)

    def _flush(self):
        if self.chain:
            self.k += 1
            lab = "m%d" % self.k
            self.fc.append("[%s]%s[%s]" % (self.cur, ",".join(self.chain), lab))
            self.cur, self.chain = lab, []

    def add_input(self, args):
        self.inputs += args
        self.nin += 1
        return self.nin - 1

    def overlay(self, idx, pre, x, y, enable=None):
        self._flush()
        self.k += 1
        pl, lab = "p%d" % self.k, "m%d" % self.k
        self.fc.append("[%d:v]%s[%s]" % (idx, pre, pl))
        e = ":enable='%s'" % enable if enable else ""
        self.fc.append("[%s][%s]overlay=x=%s:y=%s%s[%s]" % (self.cur, pl, x, y, e, lab))
        self.cur = lab

    # ---- primitives
    def _tf(self, s):
        self.n += 1
        p = self.tmp / ("%s_%04d.txt" % (self.name, self.n))
        p.write_text(s)
        return p

    def track(self, t0, y, tick=False):
        if tick:
            return
        self.first_t0 = t0 if self.first_t0 is None else min(self.first_t0, t0)
        self.last_t0 = max(self.last_t0, t0)
        self.rows.add(y)

    def text(self, s, x, y, size=34, color=TXT, font=SANS, t0=0.0, t1=None, anim="up", tick=False,
             expansion=False, box=None, alpha=1.0, enable=None):
        """x: int | 'c' (centered) | ('r', X) right-aligned at X. anim: up | left | fade | cut."""
        if not str(s).strip():
            return
        tf = self._tf(s)
        e = "clip((t-%.3f)/0.4,0,1)" % t0
        if anim == "cut":
            e = "1"
        a = e if alpha == 1.0 else "%s*%.3f" % (e, alpha)
        if t1 is not None and anim != "cut":
            a = "%s*clip((%.3f-t)/0.3,0,1)" % (a, t1)
        ya = "%d" % y
        if font in (MONO, MONOB):  # align baselines of tokens on one line (drawtext places by glyph extents)
            ya = "%d+0.76*%d-ascent" % (y, size)
        yx = "'%s'" % ya
        xx = x
        if anim == "up":
            yx = "'%s+pow(1-%s,2)*26'" % (ya, e)
        if x == "c":
            xx = "(w-text_w)/2"
        elif isinstance(x, tuple):
            xx = "%d-text_w" % x[1]
        if anim == "left":
            xx = "'%s-pow(1-%s,2)*70'" % (xx if isinstance(xx, str) else str(xx), e)
        elif isinstance(xx, str):
            xx = "'%s'" % xx
        en = "gte(t,%.3f)" % t0
        if t1 is not None:
            en += "*lt(t,%.3f)" % t1
        if enable:
            en = "(%s)*(%s)" % (en, enable)
        flt = "drawtext=fontfile=%s:textfile=%s:fontsize=%d:fontcolor=%s:x=%s:y=%s:alpha='%s':enable='%s'" % (
            font, tf, size, color, xx, yx, a, en)
        if not expansion:
            flt += ":expansion=none"
        if box:
            flt += ":box=1:boxcolor=%s:boxborderw=%d" % box
        self.f(flt)
        self.track(t0, y, tick)

    def box(self, x, y, w, h, color, t0=0.0, t1=None, fill=True, thick=3, fade=True, enable=None):
        """Filled or outlined box; a filled box fades in over three steps (stacked alpha)."""
        def one(c, a_t0):
            en = "gte(t,%.3f)" % a_t0
            if t1 is not None:
                en += "*lt(t,%.3f)" % t1
            if enable:
                en = "(%s)*(%s)" % (en, enable)
            return "drawbox=x=%d:y=%d:w=%d:h=%d:color=%s:t=%s:enable='%s'" % (
                x, y, w, h, c, "fill" if fill else str(thick), en)
        if fill and fade and "@" in color:
            base, al = color.split("@")
            al = float(al)
            part = 1 - (1 - al) ** (1 / 3.0)
            for i in range(3):
                self.f(one("%s@%.3f" % (base, part), t0 + i * 0.08))
        else:
            self.f(one(color, t0))

    def panel(self, x, y, w, h, t0=0.0, t1=None, title=None, kind="code", border=LINE):
        self.box(x + 6, y + 8, w, h, "0x000000@0.35", t0, t1)  # drop shadow
        self.box(x, y, w, h, PANEL + "@0.94", t0, t1)
        self.box(x, y, w, h, border, t0, t1, fill=False, thick=2)
        if title is not None:
            self.box(x, y, w, 44, PANEL2 + "@0.98", t0, t1)
            self.box(x, y + 44, w, 2, LINE, t0, t1)
            if kind == "term":
                for i, c in enumerate((ERR, "0xffbd2e", OK)):
                    self.text("●", x + 20 + i * 30, y + 6, 26, c, SYM, t0=t0, t1=t1, anim="fade", tick=True)
            self.text(title, x + (120 if kind == "term" else 20), y + 8, 26, DIM, MONO, t0=t0, t1=t1, anim="fade", tick=True)

    # ---- composite pieces
    def chrome(self, title, chapter, idx, nchap):
        self.text("%02d" % idx, 80, 40, 26, ACC, MONOB, t0=0.0, anim="fade", tick=True)
        self.text(chapter.upper(), 128, 40, 26, DIM, MONO, t0=0.0, anim="fade", tick=True)
        self.text(title, 80, 76, 62, TXT, SANSB, t0=0.1, anim="up", tick=True)
        self.box(80, 156, 110, 5, ACC, 0.1, fade=False)
        for i in range(nchap):
            col = ACC if i + 1 == idx else (FAINT if i + 1 < idx else LINE)
            self.text("●", 1920 - 80 - (nchap - i) * 34, 40, 26, col, SYM, t0=0.0, anim="cut", tick=True)

    def caption(self, s, t0=0.8, t1=None):
        t1 = self.dur - 0.3 if t1 is None else t1
        self.box(60, 936, 1800, 84, "0x000000@0.62", t0, t1)
        self.box(60, 936, 7, 84, ACC, t0, t1, fade=False)
        self.text(s, 96, 958, 36, TXT, SANS, t0=t0 + 0.1, t1=t1, anim="fade", tick=True)
        self.track(t0, 958)

    def code(self, lines, x, y, w, size=28, lh=40, t0=0.0, per=0.14, title=None, numbers=None, t1=None,
             anim="fade", color_fn=None, panel=True):
        """Syntax-highlighted code panel. Returns {(line, col): (x, y)} helper via closure."""
        pad = 20
        gut = 0 if numbers is None else int(cw(size) * 4 + 8)
        top = 54 if title is not None else 14
        h = top + len(lines) * lh + pad
        if panel:
            self.panel(x, y, w, h, t0, t1, title)
        for i, ln in enumerate(lines):
            ly = y + top + i * lh
            lt0 = t0 + 0.2 + i * per
            if numbers is not None:
                nn = numbers[i] if isinstance(numbers, (list, tuple)) else numbers + i
                if nn:
                    self.text("%3d" % nn, x + pad, ly, size, FAINT, MONO, t0=lt0, t1=t1, anim="fade", tick=True)
            toks = tokenize(ln) if color_fn is None else [(0, ln.rstrip(), color_fn(ln))]
            for col, s, c in toks:
                self.text(s, int(x + pad + gut + round(col * cw(size))), ly, size, c, MONO, t0=lt0, t1=t1, anim=anim, tick=True)
            self.track(lt0, ly)
        origin = (x + pad + gut, y + top)
        return lambda li, col: (int(origin[0] + round(col * cw(size))), origin[1] + li * lh), h

    def term(self, lines, x, y, w, size=28, lh=40, t0=0.0, per=0.2, title="terminal", t1=None, maxc=None, colour=None):
        maxc = maxc or int((w - 40) / cw(size))
        pad = 20
        shown = []
        for l in lines:
            l = l.replace("\t", "    ").rstrip()
            shown.append(l if len(l) <= maxc else l[:maxc - 1] + "…")
        h = 54 + len(shown) * lh + pad
        self.panel(x, y, w, h, t0, t1, title, kind="term")
        for i, l in enumerate(shown):
            lt0 = t0 + 0.2 + i * per
            c = colour(l) if colour else term_colour(l)
            if l.startswith("$ "):
                self.text("$", x + pad, y + 54 + i * lh, size, ACC, MONOB, t0=lt0, t1=t1, anim="fade", tick=True)
                self.text(l[2:], int(x + pad + cw(size) * 2), y + 54 + i * lh, size, TXT, MONO, t0=lt0, t1=t1, anim="fade", tick=True)
            else:
                self.text(l, x + pad, y + 54 + i * lh, size, c, MONO, t0=lt0, t1=t1, anim="fade", tick=True)
            self.track(lt0, y + 54 + i * lh)
        return h

    def check(self):
        if self.first_t0 is None:
            return
        need = 3.0 + 0.2 * len(self.rows)
        have = self.dur - self.first_t0
        last = self.dur - self.last_t0
        if have + 1e-6 < need or last < 2.5:
            print("  [timing] %s: dwell %.1fs (need %.1fs for %d lines), last item %.1fs before end" % (
                self.name, have, need, len(self.rows), last), flush=True)

    # ---- render
    def render(self, outdir):
        self._flush()
        self.fc.append("[%s]format=yuv420p[out]" % self.cur)
        script = self.tmp / (self.name + ".fc")
        script.write_text(";\n".join(self.fc))
        out = Path(outdir) / (self.name + ".mp4")
        cmd = ["ffmpeg", "-y", "-loglevel", "error", "-loop", "1", "-framerate", str(FPS), "-t", "%.2f" % self.dur, "-i",
               str(self.tmp / "bg.png")] + self.inputs + ["-filter_complex_script", str(script), "-map", "[out]",
               "-t", "%.2f" % self.dur] + ENC + [str(out)]
        rc, o = sh(cmd)
        if rc:
            sys.exit("ffmpeg failed for %s:\n%s\n%s" % (self.name, " ".join(cmd), o))
        return out


# ------------------------------------------------------------------------------------ assembly
TRANS = ["fade", "slideleft", "fadeblack", "smoothleft", "wipeleft", "circleopen", "slideup", "smoothup", "fade"]


def assemble(clips, durs, final, trans=0.55, kinds=None):
    """Join clips with xfade. clips: mp4 paths, durs: their durations."""
    args, fc = [], []
    for c in clips:
        args += ["-i", str(c)]
    cur, length = "0:v", durs[0]
    for i in range(1, len(clips)):
        kind = (kinds[i - 1] if kinds else TRANS[(i - 1) % len(TRANS)])
        lab = "x%d" % i
        fc.append("[%s][%d:v]xfade=transition=%s:duration=%.2f:offset=%.3f[%s]" % (cur, i, kind, trans, length - trans, lab))
        cur, length = lab, length + durs[i] - trans
    fc.append("[%s]format=yuv420p[o]" % cur)
    cmd = ["ffmpeg", "-y", "-loglevel", "error"] + args + ["-filter_complex", ";".join(fc), "-map", "[o]", "-c:v", "libx264",
           "-preset", "medium", "-crf", "21", "-pix_fmt", "yuv420p", "-r", str(FPS), "-an", "-movflags", "+faststart", str(final)]
    rc, o = sh(cmd)
    if rc:
        sys.exit("assemble failed:\n" + o)
    return length


# ------------------------------------------------------------------------------------ Life clips
def mix(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


def life_clip(outdir, boards, rate, hold_tail=0.0, alive=(79, 195, 247), dead=(18, 24, 33), decay=0.78):
    """16x16 PPM sequence at FPS: generation k shown for 1/rate s, smooth crossfade + fading trail.
    boards: list of 16x16 0/1 rows. Returns (dir, n_frames, [(gen_index, frame_start)])."""
    outdir = Path(outdir)
    outdir.mkdir(parents=True, exist_ok=True)
    per = FPS / float(rate)
    vals = []  # intensity per generation (alive=1, trail decays)
    prev = [[0.0] * 16 for _ in range(16)]
    for b in boards:
        cur = [[1.0 if b[r][c] else prev[r][c] * decay for c in range(16)] for r in range(16)]
        vals.append(cur)
        prev = cur
    nfr = int(round(len(boards) * per + hold_tail * FPS))
    starts = []
    for k in range(nfr):
        g = min(int(k / per), len(boards) - 1)
        s = (k / per) - g
        nxt = vals[min(g + 1, len(boards) - 1)]
        e = min(1.0, s * 2.0)
        e = e * e * (3 - 2 * e)  # crossfade in the first half of the generation
        data = bytearray(b"P6\n16 16\n255\n")
        for r in range(16):
            for c in range(16):
                v = vals[g][r][c] + (nxt[r][c] - vals[g][r][c]) * e
                col = mix(dead, alive, min(1.0, v))
                if v > 0.85:
                    col = mix(col, (205, 238, 255), (v - 0.85) * 3)
                data += bytes(col)
        (outdir / ("f%04d.ppm" % k)).write_bytes(bytes(data))
        starts.append(g)
    return outdir, nfr, starts


def board_pre(size, cell_gap=6, glow=True, sigma=24, tag="a"):
    """filter string turning the 16x16 ppm stream into a tiled, glowing board of `size` px."""
    cell = size // 16
    s = cell * 16
    base = "scale=%d:%d:flags=neighbor,drawgrid=w=%d:h=%d:t=%d:c=%s,format=gbrp" % (s, s, cell, cell, cell_gap, BG0)
    if not glow:
        return base
    return base + ",split[%sa][%sb];[%sb]gblur=sigma=%d[%sg];[%sa][%sg]blend=all_mode=screen" % (tag, tag, tag, sigma, tag, tag, tag)
