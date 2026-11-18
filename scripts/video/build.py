#!/usr/bin/env python3
"""Render the fe-zk demo video from REAL command output (python stdlib + ffmpeg only).

Every number on screen is captured at render time: `fe-zk-gen`, `fe test`, `life-cli` are run
here, docs/GAS.md and docs/SAFETY.md are parsed. Only labels and captions are literal text.

Run through scripts/make-video.sh (which builds the tools first). Environment:
  VIDEO_DIR   work + output directory (default /data/fe-zk-video)
  FE          fe binary (default: fe)
  GEN_BIN     fe-zk-gen binary, LIFE_BIN life-cli binary (set by make-video.sh)
"""
import os, re, shutil, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VIDEO = Path(os.environ.get("VIDEO_DIR", "/data/fe-zk-video"))
TMP = VIDEO / "tmp"
CAP = TMP / "cap"
SCN = TMP / "scenes"
FE = os.environ.get("FE", "fe")
GEN = os.environ.get("GEN_BIN", "fe-zk-gen")
LIFE = os.environ.get("LIFE_BIN", "life-cli")
MONO = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
MONOB = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
SANS = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
SANSB = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"

W, H, FPS = 1920, 1080, 30
FS, LH, X0, Y0 = 28, 38, 48, 110  # font size, line height, left margin, first line y
MAXC = 104  # columns that fit at 28 px mono
BG, BAR = "0x0d1117", "0x161b22"
C_TXT, C_DIM, C_OK, C_ERR, C_CMD, C_HI = "0xc9d1d9", "0x8b949e", "0x3fb950", "0xff7b72", "0xe3b341", "0x79c0ff"
ENC = ["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-r", str(FPS), "-an"]


def sh(cmd, cwd=None, env=None, check=False):
    """Run a command, return merged stdout+stderr text (and rc)."""
    e = dict(os.environ)
    if env:
        e.update(env)
    p = subprocess.run(cmd, cwd=cwd, env=e, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if check and p.returncode:
        sys.exit("command failed (%d): %s\n%s" % (p.returncode, " ".join(map(str, cmd)), p.stdout))
    return p.returncode, p.stdout


def capture(name, cmd, **kw):
    f = CAP / (name + ".txt")
    if os.environ.get("VIDEO_REUSE") and f.exists():  # debugging aid: reuse captures of a previous run
        out = f.read_text()
        return (1 if re.search(r"FAILED|error", out) else 0), out
    rc, out = sh(cmd, **kw)
    (CAP / (name + ".txt")).write_text(out)
    return rc, out


def log(*a):
    print("[video]", *a, flush=True)


# ---------------------------------------------------------------------------------- text scenes
def colour(line):
    s = line.strip()
    if s.startswith("$ "):
        return C_CMD
    if re.search(r"error|FAILED|FAIL |static assertion failed|panicked", line):
        return C_ERR
    if re.search(r"\bPASS\b|test result: ok|VALID|^\s*ok\b", line):
        return C_OK
    if s.startswith("#") or s.startswith("//"):
        return C_DIM
    return C_TXT


def clip(s, n=MAXC):
    s = s.replace("\t", "    ").rstrip()
    return s if len(s) <= n else s[: n - 1] + "…"


def esc_path(p):
    return str(p).replace(":", r"\:")


def textfile(name, text):
    p = SCN / name
    p.write_text(text)
    return p


class Scene:
    def __init__(self, name, title, lines=(), images=(), dur=None, hold=4.0, per_line=0.25):
        self.name, self.title, self.images = name, title, list(images)
        # lines: str or (str, colour) or (str, colour, font) ; clipped to the screen width
        self.lines = []
        for l in lines:
            if isinstance(l, str):
                l = (l, colour(l))
            self.lines.append((clip(l[0]),) + tuple(l[1:]))
        self.reveal = per_line * len(self.lines)
        self.dur = dur if dur else hold + self.reveal
        self.per_line = per_line


def drawtext(font, textf, size, color, x, y, enable=None):
    if "," in str(y):
        y = "'%s'" % y
    f = "drawtext=fontfile=%s:textfile=%s:fontsize=%d:fontcolor=%s:x=%s:y=%s:expansion=none" % (
        font, esc_path(textf), size, color, x, y)
    if enable:
        f += ":enable='%s'" % enable
    return f


def header_filters(title, dur):
    tf = textfile("t_%d.txt" % abs(hash(title)), title)
    return [
        "drawbox=x=0:y=0:w=%d:h=84:color=%s:t=fill" % (W, BAR),
        drawtext(SANSB, tf, 38, C_HI, X0, 22),
    ]


def run_ffmpeg(args, out):
    cmd = ["ffmpeg", "-y", "-loglevel", "error"] + args + ENC + [str(out)]
    p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if p.returncode:
        sys.exit("ffmpeg failed:\n" + " ".join(cmd) + "\n" + p.stdout)


def finish_chain(chain, dur):
    return chain + ["fade=t=in:st=0:d=0.3", "fade=t=out:st=%.2f:d=0.3" % (dur - 0.3)]


def render_text(sc):
    out = SCN / (sc.name + ".mp4")
    chain = header_filters(sc.title, sc.dur)
    for i, ln in enumerate(sc.lines):
        text, col = ln[0], ln[1]
        if not text.strip():
            continue
        tf = textfile("%s_%02d.txt" % (sc.name, i), text)
        t0 = 0.3 + i * sc.per_line
        chain.append(drawtext(MONO, tf, FS, col, X0, Y0 + i * LH, "gte(t,%.2f)" % t0))
    inputs = ["-f", "lavfi", "-i", "color=c=%s:s=%dx%d:r=%d:d=%.2f" % (BG, W, H, FPS, sc.dur)]
    fc = ["[0:v]" + ",".join(chain) + "[base]"]
    last = "base"
    for k, (path, x, y, size, t0) in enumerate(sc.images, 1):
        inputs += ["-i", str(path)]
        cell = size // 16
        fc.append("[%d:v]scale=%d:%d:flags=neighbor,drawgrid=w=%d:h=%d:t=1:c=0x30363d[im%d]" % (k, size, size, cell, cell, k))
        fc.append("[%s][im%d]overlay=x=%d:y=%d:enable='gte(t,%.2f)'[o%d]" % (last, k, x, y, t0, k))
        last = "o%d" % k
    fc.append("[%s]%s[out]" % (last, ",".join(finish_chain([], sc.dur)[0:2])))
    script = SCN / (sc.name + ".fc")
    script.write_text(";\n".join(fc))
    run_ffmpeg(inputs + ["-filter_complex_script", str(script), "-map", "[out]", "-t", "%.2f" % sc.dur], out)
    return out


def render_scroll(name, title, lines, speed=190.0):
    """Fast scroll of a long listing, last line ends at the bottom."""
    lines = [clip(l) for l in lines]
    n = len(lines)
    total = n * LH
    end_y = H - 40 - total
    dur = (H - 100 - end_y) / speed + 3.0
    tf = textfile(name + "_scroll.txt", "\n".join(lines))
    y = "max(%d-t*%.1f,%d)" % (H - 100, speed, end_y)
    chain = [
        "%s:line_spacing=%d" % (drawtext(MONO, tf, FS, C_TXT, X0, y), LH - 34),
    ] + header_filters(title, dur)
    out = SCN / (name + ".mp4")
    inputs = ["-f", "lavfi", "-i", "color=c=%s:s=%dx%d:r=%d:d=%.2f" % (BG, W, H, FPS, dur)]
    run_ffmpeg(inputs + ["-vf", ",".join(finish_chain(chain, dur)), "-t", "%.2f" % dur], out)
    return out, dur


def render_title(name, big, lines, dur=5.0):
    chain = []
    tf = textfile(name + "_big.txt", big)
    chain.append(drawtext(SANSB, tf, 120, C_HI, "(w-text_w)/2", 250))
    y = 450
    for i, (t, size, col) in enumerate(lines):
        f = textfile("%s_l%d.txt" % (name, i), t)
        chain.append(drawtext(SANS, f, size, col, "(w-text_w)/2", y))
        y += int(size * 1.6)
    out = SCN / (name + ".mp4")
    inputs = ["-f", "lavfi", "-i", "color=c=%s:s=%dx%d:r=%d:d=%.2f" % (BG, W, H, FPS, dur)]
    run_ffmpeg(inputs + ["-vf", ",".join(finish_chain(chain, dur)), "-t", "%.2f" % dur], out)
    return out


# ---------------------------------------------------------------------------------- Life boards
def parse_board(lines):
    """life-cli prints a 16x16 board as '. ' (dead) / '##' (alive) pairs; return 16 rows of 0/1."""
    rows = []
    for l in lines:
        s = l.rstrip("\n")
        if len(s) >= 31 and set(s) <= set(". #"):
            s = s.ljust(32)
            rows.append([1 if s[2 * i] == "#" else 0 for i in range(16)])
    return rows[:16] if len(rows) >= 16 else None


def write_ppm(path, rows, alive=(0x3F, 0xB9, 0x50), dead=(0x16, 0x1B, 0x22)):
    data = bytearray(b"P6\n16 16\n255\n")
    for r in rows:
        for c in r:
            data += bytes(alive if c else dead)
    Path(path).write_bytes(bytes(data))


def bits_to_rows(h):
    v = int(h, 16)  # bit index = row * 16 + col
    return [[(v >> (r * 16 + c)) & 1 for c in range(16)] for r in range(16)]


def life_frames(gens, pattern="rpentomino"):
    """One life-cli run with --frames-json: boards and step gas as returned by the Fe contract in revm."""
    import json
    fdir = TMP / "frames"
    shutil.rmtree(fdir, ignore_errors=True)
    fdir.mkdir(parents=True)
    jpath = TMP / "life_play.json"
    rc, out = capture("life_play", [LIFE, "--pattern", pattern, "--gens", str(gens), "--no-anim", "--frames-json", str(jpath)])
    if rc:
        sys.exit("life-cli failed:\n" + out)
    d = json.loads(jpath.read_text())
    meta = []
    for k, f in enumerate(d["frames"]):
        rows = bits_to_rows(f["board"])
        write_ppm(fdir / ("f%03d.ppm" % k), rows)
        meta.append((f["generation"], sum(map(sum, rows)), f["step_gas"]))
    return fdir, meta


def render_life_anim(gens, rate=8, min_dur=10.0):
    fdir, meta = life_frames(gens)
    nfr = len(meta)
    dur = max(nfr / rate + 1.5, min_dur)
    size = 800
    chain = header_filters("zk Game of Life: 16x16 torus on one u256, stepped by the Fe contract", dur)
    pre = ["drawgrid=w=1:h=1:t=0"]  # placeholder, replaced below
    # per-frame caption on the right
    for i, (g, alive, gas) in enumerate(meta):
        t0, t1 = i / rate, (i + 1) / rate
        cap = "generation %d / %d\nalive cells  %d\nstep() gas   %s" % (g, gens, alive, gas if gas else "-")
        tf = textfile("life_cap_%03d.txt" % i, cap)
        en = "gte(t,%.4f)" % t0 if i == nfr - 1 else "between(t,%.4f,%.4f)" % (t0, t1 - 0.0001)
        chain.append("%s:line_spacing=18" % drawtext(MONO, tf, 40, C_OK, 1000, 280, en))
    notes = [
        ("Life.step(board): word-parallel bitwise adders,", C_TXT),
        ("no per-cell loops, no storage.", C_TXT),
        ("Boards on the left come from running the compiled", C_DIM),
        ("Fe contract bytecode in revm, one call per", C_DIM),
        ("generation (life-cli --pattern rpentomino).", C_DIM),
    ]
    for i, (t, col) in enumerate(notes):
        chain.append(drawtext(MONO, textfile("life_note_%d.txt" % i, t), 28, col, 1000, 520 + i * LH))
    inputs = ["-f", "lavfi", "-i", "color=c=%s:s=%dx%d:r=%d:d=%.2f" % (BG, W, H, FPS, dur),
              "-framerate", str(rate), "-i", str(fdir / "f%03d.ppm")]
    fc = [
        "[0:v]%s[base]" % ",".join(chain),
        "[1:v]scale=%d:%d:flags=neighbor,drawgrid=w=%d:h=%d:t=1:c=0x30363d[board]" % (size, size, size // 16, size // 16),
        "[base][board]overlay=x=120:y=150[o]",
        "[o]%s[out]" % ",".join(finish_chain([], dur)),
    ]
    script = SCN / "life_anim.fc"
    script.write_text(";\n".join(fc))
    out = SCN / "life_anim.mp4"
    run_ffmpeg(inputs + ["-filter_complex_script", str(script), "-map", "[out]", "-t", "%.2f" % dur], out)
    return out, meta


# ---------------------------------------------------------------------------------- doc parsers
def md_table(text, heading):
    """Rows (list of cell lists) of the first table after a '## heading' line."""
    m = re.search(r"^## " + re.escape(heading) + r".*$", text, re.M)
    if not m:
        return []
    rows, seen = [], False
    for l in text[m.end():].splitlines():
        if l.startswith("## "):
            break
        if l.startswith("|"):
            seen = True
            cells = [c.strip() for c in l.strip().strip("|").split("|")]
            if not set("".join(cells)) <= set("-: "):
                rows.append(cells)
        elif seen:
            break
    return rows[1:]  # drop header row


def gas_lines():
    text = (ROOT / "docs/GAS.md").read_text()
    out = ["%-68s %9s %9s %7s" % ("same proof, same revm: exec gas", "Fe", "Solidity", "Fe/Sol"), ""]
    for r in md_table(text, "Verifier gas, same proof"):
        out.append("%-68s %9s %9s %7s" % (re.sub(r", N=\d+", "", r[0] + " " + r[1])[:68], r[2], r[3], r[4]))
    out.append("")
    out.append("%-68s %9s %9s %7s" % ("Poseidon (circomlibjs contract vs Fe)", "Fe", "circomlib", "Fe/ref"))
    for r in md_table(text, "Poseidon hash"):
        out.append("%-68s %9s %9s %7s" % ("Poseidon, %s inputs" % r[0], r[1], r[2], r[3]))
    for r in md_table(text, "Game of Life"):
        if r[0] == "512":
            out.append("")
            out.append("Life, 512 generations: 512 x step() %s, run() %s, one Groth16 proof %s" % (r[1], r[3], r[5]))
    return out


def safety_lines():
    text = (ROOT / "docs/SAFETY.md").read_text()
    parts = re.split(r"^## (\d+)\. (.*)$", text, flags=re.M)
    out = ["%-3s %-37s %-33s %s" % ("#", "rule", "enforced at", "tested by")]
    for i in range(1, len(parts) - 2, 3):
        n, title, body = parts[i], parts[i + 1], parts[i + 2]
        if int(n) > 9:
            break
        row = next((l for l in body.splitlines() if l.startswith("|") and "`" in l), "")
        cells = [c.strip() for c in row.strip("|").split("|")]
        where = re.findall(r"`([^`]+)`", cells[0])[0] if cells and re.findall(r"`([^`]+)`", cells[0]) else "-"
        toks = re.findall(r"`([^`]+)`", cells[2]) if len(cells) > 2 else []
        test = next((t for t in toks if "/" not in t), toks[0] if toks else "-")
        title = re.split(r"[:,]| are | of every| every", title)[0].strip()
        out.append("%-3s %-37s %-33s %s" % (n, title[:37], where[:33], test))
    return out


# ---------------------------------------------------------------------------------- tree scene
TREE_LABELS = {
    "": "fe_zk ingot (Fe 26.5.0)",
    "lib.fe": "re-exports",
    "field": "Field trait + 5 fields",
    "field/fp.fe": "Fp<P>: Bn254Fr, Bn254Fq, KoalaBear, BabyBear, Goldilocks",
    "field/ext.fe": "extension fields (KoalaBear degree 4)",
    "ec": "curves and precompiles",
    "ec/bn254.fe": "BN254 G1/G2: curve + subgroup checks, pairing_check",
    "ec/bls12_381.fe": "BLS12-381 wrappers (EIP-2537)",
    "ec/eip4844.fe": "EIP-4844 point evaluation (0x0a)",
    "ec/precompile.fe": "ecAdd / ecMul / ecPairing, checked results",
    "hash": "hashes and transcripts, generic over Field",
    "hash/poseidon.fe": "Poseidon (circomlib-compatible, BN254)",
    "hash/poseidon2.fe": "Poseidon2 (BN254 t=3, KoalaBear t=16)",
    "hash/transcript.fe": "Keccak Fiat-Shamir transcript",
    "verifier": "proof verifiers",
    "verifier/groth16.fe": "Groth16 (snarkjs / arkworks / gnark keys)",
    "verifier/plonk.fe": "PLONK (snarkjs)",
    "verifier/gnark_plonk.fe": "PLONK (gnark)",
    "verifier/halo2": "halo2-KZG (SHPLONK + GWC)",
    "verifier/kzg.fe": "KZG: single, GWC, SHPLONK openings",
    "verifier/fri": "FRI / STARK (Plonky3, KoalaBear)",
}


def tree_lines():
    src = ROOT / "src"
    files = sorted(
        p.relative_to(src).as_posix()
        for p in src.rglob("*.fe")
        if "tests" not in p.relative_to(src).parts and p.name != "tests.fe"
        and not re.search(r"(_constants|_unrolled)\.fe$", p.name)
    )
    # top-level facade files (field.fe, ec.fe, ...) are just module roots of the directories
    files = [f for f in files if "/" in f or f == "lib.fe"]
    tree = {}
    for f in files:
        node = tree
        parts = f.split("/")
        for d in parts[:-1]:
            node = node.setdefault(d, {})
        node[parts[-1]] = None
    out = ["src/".ljust(34) + "# " + TREE_LABELS[""]]

    def walk(node, prefix, rel):
        items = sorted(node.items(), key=lambda kv: (kv[1] is None, kv[0]))
        for i, (name, sub) in enumerate(items):
            last = i == len(items) - 1
            r = (rel + "/" + name) if rel else name
            lab = TREE_LABELS.get(r)
            if sub is not None:
                r2 = r
                lab = TREE_LABELS.get(r2)
            nm = prefix + ("└── " if last else "├── ") + name + ("/" if sub is not None else "")
            out.append((nm.ljust(34) + ("# " + lab if lab else "")).rstrip())
            if sub is not None:
                walk(sub, prefix + ("    " if last else "│   "), r)

    walk(tree, "", "")
    return out


# ---------------------------------------------------------------------------------- main
def main():
    reuse = bool(os.environ.get("VIDEO_REUSE"))
    for d in (TMP.iterdir() if TMP.exists() else []):
        if not (reuse and d.name in ("cap", "gen")):
            shutil.rmtree(d, ignore_errors=True)
    for d in (TMP, CAP, SCN):
        d.mkdir(parents=True, exist_ok=True)
    t_start = time.time()
    scenes = []
    fe_ver = sh([FE, "--version"])[1].strip()
    rel = lambda p: os.path.relpath(ROOT, p)

    # 1. title
    scenes.append(render_title("s01_title", "fe-zk", [
        ("Zero-knowledge proof verifiers for Fe", 52, C_TXT),
        ("Fe contracts verify Groth16, PLONK, halo2-KZG, KZG and FRI/STARK proofs,", 30, C_DIM),
        ("and a generator turns verifying keys into safe Fe verifier contracts.", 30, C_DIM),
        ("Fe Builders Challenge 2026", 30, C_HI),
    ]))

    # 2. the library (scroll of the real src/ tree)
    log("scene 2: library tree")
    p, d = render_scroll("s02_library", "The library: src/ module tree", tree_lines())
    scenes.append(p)

    # 3. generator
    log("scene 3: generator")
    gdir = TMP / "gen"
    gdir.mkdir(exist_ok=True)
    V = ROOT / "vectors/groth16/membership"
    gm = gdir / "membership_verifier"
    cmd = [GEN, "groth16", "--snarkjs", str(V / "verification_key.json"), "--proof", str(V / "proof.json"),
           "--public", str(V / "public.json"), "--name", "Membership", "--out", str(gm),
           "--fe-zk-path", os.path.relpath(ROOT, gm)]
    rc, out_g = capture("gen_groth16", cmd)
    assert rc == 0, out_g
    shown = "$ fe-zk-gen groth16 --snarkjs vectors/groth16/membership/verification_key.json \\"
    gl = [shown, "      --proof proof.json --public public.json --name Membership --out membership_verifier"]
    gl += out_g.rstrip().splitlines()
    gl.append("")
    P = ROOT / "vectors/plonk/membership"
    pdir = gdir / "plonk_verifier"
    rc, out_p = capture("gen_plonk", [GEN, "plonk", "--snarkjs", str(P / "verification_key.json"), "--proof", str(P / "proof.json"),
                                      "--public", str(P / "public.json"), "--name", "PlonkMembership", "--out", str(pdir),
                                      "--fe-zk-path", os.path.relpath(ROOT, pdir)])
    assert rc == 0, out_p
    gl.append("$ fe-zk-gen plonk --snarkjs vectors/plonk/membership/verification_key.json ...")
    gl += out_p.rstrip().splitlines()
    gl.append("")
    Hh = ROOT / "vectors/halo2/standard_plonk_lookup"
    hdir = gdir / "halo2_verifier"
    rc, out_h = capture("gen_halo2", [GEN, "halo2", "--vk", str(Hh / "vk.json"), "--shplonk", str(Hh / "shplonk.json"),
                                      "--gwc", str(Hh / "gwc.json"), "--name", "Halo2Circuit", "--out", str(hdir),
                                      "--fe-zk-path", os.path.relpath(ROOT, hdir)])
    assert rc == 0, out_h
    gl.append("$ fe-zk-gen halo2 --vk vectors/halo2/standard_plonk_lookup/vk.json --shplonk ... --gwc ...")
    gl += out_h.rstrip().splitlines()
    scenes.append(render_text(Scene("s03a_gen", "Generator: fe-zk-gen validates the key, then emits a Fe verifier", gl)))

    lib = (gm / "src/lib.fe").read_text().splitlines()
    head = lib[:12]
    sa = [l for l in lib if l.startswith("static_assert")][:5]
    gl2 = ["$ head -n 12 membership_verifier/src/lib.fe"] + head
    gl2 += ["", "$ grep static_assert membership_verifier/src/lib.fe"] + sa
    scenes.append(render_text(Scene("s03b_head", "Generated verifier: header with key digest, compile-time key checks", gl2, per_line=0.2)))

    log("   fe test in generated ingot (slow on a cold cache)")
    rc, out_t = capture("fe_test_generated", [FE, "test"], cwd=gm)
    tl = ["$ cd membership_verifier && fe test"] + [l for l in out_t.splitlines() if l.strip()][-20:]
    scenes.append(render_text(Scene("s03c_test", "fe test in the generated ingot: valid proof accepted, invalid battery rejected", tl, per_line=0.2)))

    # 4. compile-time failure on a corrupted key
    log("scene 4: corrupted key")
    bad = gdir / "bad_verifier"
    shutil.rmtree(bad, ignore_errors=True)
    shutil.copytree(gm, bad, ignore=shutil.ignore_patterns("out"))
    text = (bad / "src/lib.fe").read_text()
    m = re.search(r"^pub const BETA_X0: u256 = \d*(\d)$", text, re.M)
    digit, new = m.group(1), str((int(m.group(1)) + 1) % 10)
    sed = ["sed", "-i", "-E", "/^pub const BETA_X0/ s/%s$/%s/" % (digit, new), str(bad / "src/lib.fe")]
    subprocess.run(sed, check=True)
    rc, out_b = capture("fe_test_bad", [FE, "test"], cwd=bad)
    assert rc != 0 and "static assertion failed" in out_b, out_b
    bl = ["$ sed -i -E '/^pub const BETA_X0/ s/%s$/%s/' src/lib.fe    # change one digit of a vk coordinate" % (digit, new),
          "$ fe test"] + [l for l in out_b.splitlines() if l.strip()][:20]
    scenes.append(render_text(Scene("s04_bad", "Fe catches a bad verifying key at compile time", bl, per_line=0.2)))

    # 5. quickstart
    log("scene 5: quickstart")
    q = ROOT / "examples/quickstart"
    toml = [l for l in (q / "fe.toml").read_text().splitlines() if l.startswith(("fe_zk", "multiplier2_verifier", "[dep"))]
    src = (q / "src/lib.fe").read_text().splitlines()

    def grab(pat, n):
        i = next(i for i, l in enumerate(src) if pat in l)
        return src[i:i + n]

    def dedent(ls):
        k = min(len(l) - len(l.lstrip()) for l in ls if l.strip())
        return [l[k:] for l in ls]

    ql = ["$ cat examples/quickstart/fe.toml    # [dependencies]"] + toml
    ql += ["// examples/quickstart/src/lib.fe"]
    ql += [l for l in src if l.startswith("use ") and "std::" not in l]
    ql += grab("pub fn horner", 9)
    ql += dedent(grab("poseidon_2(v, s)", 1))
    ql += dedent(grab("store\n", 1) if False else [src[next(i for i, l in enumerate(src) if "Check { a, b, c, input }" in l) + 1 + j] for j in range(5)])
    scenes.append(render_text(Scene("s05a_quick", "Use it in your own Fe code: examples/quickstart", ql, per_line=0.2)))
    rc, out_q = capture("fe_test_quickstart", [FE, "test"], cwd=q)
    qt = ["$ cd examples/quickstart && fe test"] + [l for l in out_q.splitlines() if l.strip()][-8:]
    scenes.append(render_text(Scene("s05b_quicktest", "Valid proof accepted, tampered rejected, Poseidon = circomlibjs, one generic helper, two fields", qt)))

    # 6. Life
    log("scene 6: life (life-cli --frames-json)")
    anim, meta = render_life_anim(64)
    scenes.append(anim)
    import json
    pj = TMP / "life_prove.json"
    rc, out_l = capture("life_prove", [LIFE, "--prove", "--frames-json", str(pj)])
    assert rc == 0, out_l
    lines = out_l.splitlines()
    idx_end = next(i for i, l in enumerate(lines) if l.startswith("end board"))
    pd = json.loads(pj.read_text())
    start_rows = bits_to_rows(pd["start_board"])
    end_rows = bits_to_rows(pd["end_board"])
    write_ppm(TMP / "prove_start.ppm", start_rows)
    write_ppm(TMP / "prove_end.ppm", end_rows)
    gens = re.search(r"proven generations (\d+)", out_l).group(1)
    s_alive = re.search(r"start board\s+alive (\d+)", out_l).group(1)
    e_alive = re.search(r"end board after (\d+) generations.*alive (\d+)", out_l)
    rest = [l for l in lines[idx_end + 17:] if l.strip()]
    below = list(rest)
    ims = [(TMP / "prove_start.ppm", 360, 130, 384, 0.5), (TMP / "prove_end.ppm", 1176, 130, 384, 1.0)]
    cap = " " * 18 + ("start (alive %s)" % s_alive).ljust(48) + "end, gen %s (alive %s)" % (e_alive.group(1), e_alive.group(2))
    body = [("", C_TXT)] * 11 + [(cap, C_DIM), ("", C_TXT)] + [(l, colour(l)) for l in ["$ life-cli --prove"] + below]
    sc = Scene("s06b_prove", "One Groth16 proof of %s generations, verified on chain" % gens, body, images=ims, hold=5.0)
    sc.reveal = sc.dur = 0
    sc.dur = 5.0 + 0.25 * len([l for l in body if l[0].strip()])
    scenes.append(render_text(sc))

    # 7. safety
    log("scene 7-8: safety, gas")
    scenes.append(render_text(Scene("s07_safety", "Safety: every rule has an enforcement point and a test that fails without it", safety_lines(), per_line=0.35)))

    # 8. gas
    scenes.append(render_text(Scene("s08_gas", "Gas vs Solidity reference verifiers (docs/GAS.md, scripts/gas-table.sh)", gas_lines(), per_line=0.35)))

    # 9. outro
    scenes.append(render_title("s09_outro", "fe-zk", [
        ("github.com/0xitsss/fe-zk", 48, C_TXT),
        (fe_ver, 34, C_DIM),
        ("Apache-2.0", 34, C_DIM),
        ("Verifiers, generator, safety rules and tests: all in Fe", 30, C_HI),
    ]))

    # concat
    lst = SCN / "concat.txt"
    lst.write_text("".join("file '%s'\n" % s for s in scenes))
    final = VIDEO / "fe-zk-demo.mp4"
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", str(lst), "-c", "copy",
                    "-movflags", "+faststart", str(final)], check=True)
    # teaser: first 10 s of the Life animation
    gif = VIDEO / "teaser.gif"
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-t", "10", "-i", str(anim), "-vf",
                    "fps=10,scale=800:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=64[p];[b][p]paletteuse",
                    str(gif)], check=True)
    log("done in %.0f s: %s, %s" % (time.time() - t_start, final, gif))


if __name__ == "__main__":
    main()
