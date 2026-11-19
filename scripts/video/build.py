#!/usr/bin/env python3
"""Render the fe-zk demo video v2 from REAL command output and repository files (stdlib + ffmpeg).

Every number, output line, board and code excerpt on screen is captured at render time:
`fe-zk-gen`, `fe test`, `fe build`, `life-cli` are run here; docs/GAS.md, docs/TESTING.md and
docs/SAFETY.md are parsed; code excerpts are cut from src/ and the examples by pattern with their
real line numbers. Only labels, titles and captions are literal text (see scene list in
docs/AI_LOG.md / the final report).

Run through scripts/make-video.sh (which builds the tools first). Environment:
  VIDEO_DIR    work + output directory (default /data/fe-zk-video)
  FE           fe binary (default: fe)
  GEN_BIN      fe-zk-gen binary, LIFE_BIN life-cli binary (set by make-video.sh)
  VIDEO_REUSE  debugging aid: reuse captures of the previous run (VIDEO_DIR/tmp/cap)
  VIDEO_ONLY   comma separated scene names to render (skips assembly)
"""
import json, os, re, shutil, subprocess, sys, textwrap, time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from fx import *  # noqa

ROOT = Path(__file__).resolve().parents[2]
VIDEO = Path(os.environ.get("VIDEO_DIR", "/data/fe-zk-video"))
TMP = VIDEO / "tmp"
CAP = TMP / "cap"
CLIPS = TMP / "clips"
SRC = TMP / "scn"
FE = os.environ.get("FE", "fe")
GEN = os.environ.get("GEN_BIN", "fe-zk-gen")
LIFE = os.environ.get("LIFE_BIN", "life-cli")
NCHAP = 10


def log(*a):
    print("[video]", *a, flush=True)


def capture(name, cmd, always=False, **kw):
    f = CAP / (name + ".txt")
    if os.environ.get("VIDEO_REUSE") and f.exists() and not always:
        out = f.read_text()
        return (1 if re.search(r"FAILED|^error", out, re.M) else 0), out
    rc, out = sh(cmd, **kw)
    f.write_text(out)
    return rc, out


def relpath(a, b):
    return os.path.relpath(a, b)


def set_dep_path(ingot):
    """Point the fe_zk dependency of a scratch ingot at this checkout (relative path, as fe requires)."""
    f = Path(ingot) / "fe.toml"
    t = re.sub(r'path = "[^"]*"', 'path = "%s"' % relpath(ROOT, ingot), f.read_text())
    f.write_text(t)


def fmt(n):
    return "{:,}".format(int(n))


def num(s):
    return int(re.sub(r"[^\d]", "", s))


# ---------------------------------------------------------------------------------- code excerpts
class Src:
    def __init__(self, path):
        self.path = ROOT / path
        self.lines = self.path.read_text().splitlines()

    def find(self, pat, start=0):
        for i in range(start, len(self.lines)):
            if pat in self.lines[i]:
                return i
        raise SystemExit("pattern not found in %s: %s" % (self.path, pat))

    def grab(self, pat, n, start=0, dedent=True, until=None):
        """n lines from the first line containing pat (or until the first line == `until`).
        Returns (lines, line numbers)."""
        i = self.find(pat, start)
        if until is not None:
            j = i
            while self.lines[j].rstrip() != until:
                j += 1
            n = j - i + 1
        ls = self.lines[i:i + n]
        if dedent:
            k = min(len(l) - len(l.lstrip()) for l in ls if l.strip())
            ls = [l[k:] for l in ls]
        return ls, list(range(i + 1, i + 1 + n))


def join(*parts):
    ls, ns = [], []
    for a, b in parts:
        ls += a
        ns += b
    return ls, ns


# ---------------------------------------------------------------------------------- doc parsers
def md_table(text, heading):
    m = re.search(r"^## " + re.escape(heading) + r".*$", text, re.M)
    if not m:
        raise SystemExit("heading not found: " + heading)
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
    return rows[1:]


def families():
    """Rows for the 'every family' table, all from docs/GAS.md."""
    text = (ROOT / "docs/GAS.md").read_text()
    gas = md_table(text, "Verifier gas, same proof")
    only = md_table(text, "Fe only")
    size = md_table(text, "Deployed code size")
    sizes = {r[0]: r for r in size}

    def grp(prefix):
        g = [r for r in gas if r[0].startswith(prefix)]
        s = [r for r in size if r[0].startswith(prefix)]
        ratios = [float(r[4].rstrip("x")) for r in g]
        return dict(gas=max(num(r[2]) for r in g), lo=min(ratios), hi=max(ratios), code=max(num(r[1]) for r in s))

    out = []
    out.append(("Groth16 (snarkjs)", grp("Groth16")))
    out.append(("PLONK (snarkjs)", grp("PLONK (snarkjs)")))
    out.append(("PLONK (gnark)", grp("PLONK (gnark")))
    out.append(("halo2-KZG", grp("halo2-KZG")))
    kz = [r for r in only if r[0] == "KZG"]
    ks = [r for r in size if r[0].startswith("KZG")]
    out.append(("KZG (single, GWC, SHPLONK)", dict(gas=max(num(r[2]) for r in kz), lo=None, hi=None, code=max(num(r[1]) for r in ks))))
    fr = [r for r in only if r[0] == "FRI/STARK"][0]
    fs = [r for r in size if r[0].startswith("FRI/STARK")][0]
    out.append(("FRI / STARK (Plonky3)", dict(gas=num(fr[2]), lo=None, hi=None, code=num(fs[1]))))
    return out


def testing_totals():
    text = (ROOT / "docs/TESTING.md").read_text()
    row = next(l for l in text.splitlines() if l.startswith("| **total**"))
    c = [x.strip().strip("*") for x in row.strip("|").split("|")]
    valid, muts, agree, allowed, bugs = [int(x) for x in c[1:6]]
    return dict(cases=valid + muts, bugs=bugs, valid=valid, muts=muts)


def safety_rules():
    text = (ROOT / "docs/SAFETY.md").read_text()
    return len([n for n in re.findall(r"^## (\d+)\. ", text, re.M) if int(n) <= 9])


# ---------------------------------------------------------------------------------- life data
def bits_to_rows(h):
    v = int(h, 16)
    return [[(v >> (r * 16 + c)) & 1 for c in range(16)] for r in range(16)]


def write_ppm(path, rows, alive=(79, 195, 247), dead=(18, 24, 33)):
    data = bytearray(b"P6\n16 16\n255\n")
    for r in rows:
        for c in r:
            data += bytes(alive if c else dead)
    Path(path).write_bytes(bytes(data))


def life_run(name, args):
    jp = TMP / (name + ".json")
    rc, out = capture(name, [LIFE] + args + ["--frames-json", str(jp)])
    if rc:
        sys.exit("life-cli failed:\n" + out)
    return json.loads(jp.read_text()), out


# ---------------------------------------------------------------------------------- helpers
def counter(sc, value, x, y, size, color, t0, dur=1.8, font=MONOB):
    p = "clip((t-%.2f)/%.2f,0,1)" % (t0, dur)
    X = "trunc(%d*(1-pow(1-%s,3)))" % (value, p)
    sc.text("%%{eif:%s:d}" % X, x, y, size, color, font, t0=t0, anim="cut", expansion=True, enable="lt(%s,1000)" % X, tick=True)
    sc.text("%%{eif:floor(%s/1000):d},%%{eif:mod(%s,1000):d:3}" % (X, X), x, y, size, color, font, t0=t0, anim="cut",
            expansion=True, enable="gte(%s,1000)" % X, tick=True)
    sc.track(t0, y)


def flash(sc, x, y, w, h, t0, color=ACC, thick=3):
    """Outline box that blinks three times, then stays on."""
    sc.box(x, y, w, h, color, t0, fill=False, thick=thick,
           enable="gt(t,%.2f)+lt(mod(t-%.2f,0.24),0.12)" % (t0 + 0.72, t0))


def grow_bar(sc, x, y, wfinal, h, color, t0, dur, step=2):
    n = max(2, int(dur * FPS / step))
    for i in range(n):
        a = t0 + i * step / float(FPS)
        e = (i + 1) / float(n)
        e = 1 - (1 - e) ** 3
        sc.f("drawbox=x=%d:y=%d:w=%d:h=%d:color=%s:t=fill:enable='%s'" % (
            x, y, max(2, int(wfinal * e)), h, color,
            ("between(t,%.3f,%.3f)" % (a, a + step / float(FPS) - 0.001)) if i < n - 1 else "gte(t,%.3f)" % a))



# ---------------------------------------------------------------------------------- main
def main():
    reuse = bool(os.environ.get("VIDEO_REUSE"))
    for d in (TMP.iterdir() if TMP.exists() else []):
        if not (reuse and d.name in ("cap",)):
            shutil.rmtree(d, ignore_errors=True)
    for d in (TMP, CAP, CLIPS, SRC):
        d.mkdir(parents=True, exist_ok=True)
    t_start = time.time()
    only = set(filter(None, os.environ.get("VIDEO_ONLY", "").split(",")))
    make_background(SRC / "bg.png", SRC)
    fe_ver = sh([FE, "--version"])[1].strip()
    log("fe:", fe_ver)

    # ------------------------------------------------------------ captures (real runs)
    log("capture: fe test over every ingot (parallel)")
    ingots = [("fe-zk library", ROOT)] + [
        (str(p.parent.relative_to(ROOT)), p.parent) for p in sorted((ROOT / "examples").rglob("fe.toml"))
        if "out" not in p.parts]

    def run_test(it):
        label, d = it
        rc, out = capture("fe_test_" + re.sub(r"\W", "_", label), [FE, "test"], cwd=d)
        m = re.search(r"test result: (\w+)\. (\d+) passed; (\d+) failed", out)
        return label, rc, out, (int(m.group(2)) if m else 0), (int(m.group(3)) if m else 99)

    with ThreadPoolExecutor(4) as ex:
        results = list(ex.map(run_test, ingots))
    n_tests = 0
    results = [r for r in results if "No tests found" not in r[2]]  # e.g. examples/fri_stark: deployable ingot, tests live in src/tests
    for label, rc, out, p, f in results:
        log("   %-40s %4d passed, %d failed" % (label, p, f))
        assert f == 0 and p > 0, "fe test failed in %s:\n%s" % (label, out[-2000:])
        n_tests += p
    quick_out = next(o for l, r, o, p, f in results if l == "examples/quickstart")
    sem_label = "examples/real_semaphore/verifier"
    sem_out = next(o for l, r, o, p, f in results if l == sem_label)

    log("capture: fe-zk-gen on the real Semaphore v4 key")
    gdir = TMP / "gen"
    gdir.mkdir()
    (gdir / "vectors").symlink_to(ROOT / "vectors")
    vk = "vectors/groth16/real_semaphore_d10"
    gcmd = [GEN, "groth16", "--snarkjs", vk + "/verification_key.json", "--proof", vk + "/proof.json", "--public",
            vk + "/public.json", "--name", "Semaphore", "--out", "semaphore_verifier", "--fe-zk-path", relpath(ROOT, gdir / "semaphore_verifier")]
    rc, gen_out = capture("gen_semaphore", gcmd, always=True, cwd=gdir)
    assert rc == 0, gen_out
    sem_dir = gdir / "semaphore_verifier"
    sem_lib = Src.__new__(Src)
    sem_lib.path, sem_lib.lines = sem_dir / "src/lib.fe", (sem_dir / "src/lib.fe").read_text().splitlines()
    digest = re.search(r"key digest\s*:\s*(0x[0-9a-f]+)", gen_out).group(1)
    committed = (ROOT / "examples/real_semaphore/verifier/src/lib.fe").read_text()
    same = digest in committed
    log("   digest", digest, "matches committed real_semaphore ingot:", same)

    log("capture: corrupted key, fe test")
    bad = TMP / "bad_semaphore"
    shutil.copytree(sem_dir, bad, ignore=shutil.ignore_patterns("out"))
    set_dep_path(bad)
    btxt = (bad / "src/lib.fe").read_text()
    m = re.search(r"^pub const BETA_X0: u256 = (\d+)$", btxt, re.M)
    old_digit = m.group(1)[-1]
    new_digit = str((int(old_digit) + 5) % 10)
    (bad / "src/lib.fe").write_text(btxt.replace(m.group(0), m.group(0)[:-1] + new_digit))
    rc, bad_out = capture("fe_test_bad", [FE, "test"], cwd=bad)
    assert rc != 0 and "static assertion failed" in bad_out, bad_out

    log("capture: raw u256 -> G1 compile error")
    probe = TMP / "g1probe"
    (probe / "src").mkdir(parents=True)
    (probe / "fe.toml").write_text('[ingot]\nname = "g1probe"\nversion = "0.1.0"\n\n[dependencies]\nfe_zk = { path = "x", allow_unsafe = true }\n')
    set_dep_path(probe)
    (probe / "src/lib.fe").write_text("use fe_zk::ec::G1\n\npub fn forge() -> G1 {\n    G1 { x: 1, y: 2 }\n}\n")
    rc, probe_out = capture("fe_build_g1probe", [FE, "build"], cwd=probe)
    assert rc != 0 and "not visible" in probe_out, probe_out

    log("capture: life-cli")
    cold, _ = life_run("life_cold", ["--pattern", "random", "--seed", "7", "--gens", "80", "--no-anim"])
    play, _ = life_run("life_play", ["--pattern", "rpentomino", "--gens", "64", "--no-anim"])
    prove, prove_out = life_run("life_prove", ["--prove"])

    # ------------------------------------------------------------ scenes
    scenes = []  # (name, Scene)

    def new(name, dur, title=None, chapter=None, idx=None):
        sc = Scene(name, dur, SRC, title, chapter, idx, NCHAP)
        scenes.append(sc)
        return sc

    # ---- 1. cold open
    sc = new("s01_cold", 9.5)
    rate = 8
    boards = [bits_to_rows(f["board"]) for f in cold["frames"]]
    d, nfr, starts = life_clip(SRC / "cold", boards, rate, hold_tail=1.0)
    i = sc.add_input(["-framerate", str(FPS), "-i", str(d / "f%04d.ppm")])
    sc.overlay(i, board_pre(1008, 6, True, 26, "c"), 620, 36)
    sc.text("GENERATION", 70, 330, 26, DIM, MONO, t0=0.3, t1=5.5, anim="fade", tick=True)
    sc.text("STEP() GAS", 70, 640, 26, DIM, MONO, t0=0.3, t1=5.5, anim="fade", tick=True)
    sc.text("Fe contract, run in revm", 70, 800, 26, DIM, MONO, t0=0.3, t1=5.5, anim="fade", tick=True)
    for g, f in enumerate(cold["frames"]):
        ta, tb = g / float(rate), (g + 1) / float(rate)
        if ta > 5.5:
            break
        sc.text("%d" % g, 70, 370, 170, ACC, MONOB, t0=ta, t1=min(tb, 5.5), anim="cut", tick=True)
        sc.text(str(f["step_gas"]) if f["step_gas"] else "-", 70, 680, 100, TXT, MONOB, t0=ta, t1=min(tb, 5.5), anim="cut", tick=True)
    for k in range(6):
        sc.box(0, 0, W, H, "0x05080c@0.2", 5.4 + k * 0.1, fill=True, fade=False)
    sc.text("fe-zk", "c", 330, 190, ACC, SANSB, t0=6.0, anim="up", tick=True)
    sc.text("zero-knowledge verifiers, written in Fe", "c", 560, 62, TXT, SANSB, t0=6.4, anim="up", tick=True)
    sc.text("Fe Builders Challenge 2026", "c", 660, 34, DIM, SANS, t0=6.9, anim="fade", tick=True)
    sc.track(0.3, 1)

    # ---- 2. the problem
    sc = new("s02_problem", 9.0, "The problem", "why", 1)
    sc.text("Fe had no proof verifier.", 80, 230, 90, TXT, SANSB, t0=0.5)
    sc.text("zk projects had to leave Fe for Solidity.", 80, 350, 64, ACC, SANSB, t0=2.2)
    cards = [("Your Fe contract", ACC, 0.8), ("Proof verifier", ERR, 3.8), ("Solidity", DIM, 5.2)]
    for k, (lab, col, t0) in enumerate(cards):
        x = 80 + k * 640
        sc.box(x, 540, 480, 190, PANEL2 + "@0.9", t0)
        sc.box(x, 540, 480, 190, col, t0, fill=False, thick=3)
        sc.text(lab, x + 30, 580, 44, TXT, SANSB, t0=t0 + 0.1, tick=True)
    sc.text("Groth16 · PLONK · halo2", 80 + 640 + 30, 636, 24, DIM, MONO, t0=3.9, tick=True)
    sc.text("KZG · FRI/STARK", 80 + 640 + 30, 664, 24, DIM, MONO, t0=3.9, tick=True)
    sc.text("none in Fe", 80 + 640 + 30, 694, 28, ERR, MONOB, t0=4.2, tick=True)
    sc.text("the only option", 80 + 1280 + 30, 650, 28, DIM, MONO, t0=5.3, tick=True)
    sc.text("→", 570, 570, 90, ACC, SYM, t0=1.6, tick=True)
    sc.text("→", 1210, 570, 90, DIM, SYM, t0=5.0, tick=True)
    sc.caption("fe-zk closes the gap: field, curve and hash layer, verifiers and a safe generator, all in Fe", 6.2)
    sc.track(0.5, 1)

    # ---- 3a. pipeline
    sc = new("s03a_pipe", 7.0, "One command", "the generator", 2)
    boxes = [("vk.json", ["snarkjs · arkworks", "gnark"]), ("fe-zk-gen", ["validates the key", "on curve, in subgroup"]),
             ("Fe verifier", ["an ingot, key inside", "static_assert checks"]), ("deployed contract", ["pure verification", "static call"])]
    for k, (a, bl) in enumerate(boxes):
        x, t0 = 80 + k * 470, 0.5 + k * 0.9
        sc.box(x, 300, 360, 210, PANEL2 + "@0.92", t0)
        sc.box(x, 300, 360, 210, ACC if k == 1 else LINE, t0, fill=False, thick=3)
        sc.text(a, x + 28, 340, 40 if k < 3 else 36, TXT, SANSB, t0=t0 + 0.1, tick=True)
        for j, l in enumerate(bl):
            sc.text(l, x + 28, 420 + j * 34, 24, DIM, MONO, t0=t0 + 0.2, tick=True)
        if k < 3:
            sc.text("→", x + 372, 372, 64, ACC, SYM, t0=t0 + 0.6, anim="left", tick=True)
    sc.term(["$ " + "fe-zk-gen groth16 --snarkjs verification_key.json --proof proof.json --public public.json"],
            80, 600, 1760, 28, 40, t0=4.0, title="one command", maxc=105)
    sc.caption("One command turns a verifying key into a safe Fe verifier ingot.", 4.4)
    sc.track(0.5, 300)

    # ---- 3b. real output on the real Semaphore key
    sc = new("s03b_real", 12.5, "Real Semaphore v4 key", "the generator", 2)
    out_lines = [l for l in gen_out.splitlines() if l.strip()]
    cmds = ["$ fe-zk-gen groth16 --snarkjs %s/verification_key.json \\" % vk, "      --proof %s/proof.json \\" % vk,
            "      --public %s/public.json \\" % vk, "      --name Semaphore --out semaphore_verifier"]
    L = cmds + out_lines
    L = [re.sub(r"/.*/gen/", "", l) if "emitted" in l else l for l in L]
    sc.term(L, 80, 190, 1760, 28, 38, t0=0.3, per=0.3, title="~/fe-zk", maxc=104)
    dig_i = next(i for i, l in enumerate(L) if "key digest" in l)
    h1 = 54 + len(L) * 38 + 20
    assert len(cmds) == 4
    flash(sc, 90, 190 + 54 + dig_i * 38 - 3, 1740, 40, 0.3 + dig_i * 0.3 + 1.6)
    head = [l for l in sem_lib.lines[:6]]
    y2 = 190 + h1 + 30
    hd = [l for l in sem_lib.lines[:4]]
    getd = next(i for i, l in enumerate(hd) if "Key digest" in l)
    co, h2 = sc.code(hd, 80, y2, 1760, 28, 38, t0=4.6, per=0.3, title="semaphore_verifier/src/lib.fe  (generated)",
                     numbers=[1, 2, 3, 4], color_fn=lambda l: DIM if "Key digest" not in l else TXT)
    flash(sc, 90, y2 + 54 + getd * 38 - 3, 1740, 40, 6.6)
    sc.caption("The key digest is embedded in the emitted code%s." % (" and matches the committed Semaphore v4 ingot" if same else ""), 7.0)

    # ---- 4a. static_assert + one digit
    sc = new("s04a_static", 11.0, "Fe catches a bad key at compile time", "compile-time safety", 3)
    ln_const = sem_lib.find("pub const BETA_X0")
    sa = [l for l in sem_lib.lines if l.startswith("static_assert(g")]
    pick = [l for l in sa if any(k in l for k in ("ALPHA", "BETA", "GAMMA", "DELTA", "IC0"))]
    code_lines = [sem_lib.lines[ln_const], ""] + pick
    nums = [ln_const + 1, 0] + [sem_lib.lines.index(l) + 1 for l in pick]
    cl = list(code_lines)
    cl[0] = cl[0][:-1]  # last digit drawn separately
    pos, h = sc.code(cl, 20, 190, 1880, 28, 42, t0=0.3, per=0.3, title="semaphore_verifier/src/lib.fe", numbers=[n or 0 for n in nums])
    gx, gy = pos(0, len(code_lines[0]) - 1)
    # blank line number cleanup: draw nothing for the blank row (number 0 would show "  0")
    sc.text(old_digit, gx, gy, 28, BLUE, MONO, t0=0.9, t1=5.6, anim="fade", tick=True)
    sc.box(gx - 3, gy - 2, 24, 40, ERR + "@0.55", 5.0, 5.6, fade=False)
    sc.box(gx - 3, gy - 2, 24, 40, ERR + "@0.55", 5.7, 6.1, fade=False)
    sc.box(gx - 3, gy - 2, 24, 40, ERR + "@0.55", 6.2, 6.6, fade=False)
    sc.text(new_digit, gx, gy, 28, ERR, MONOB, t0=5.6, anim="cut", tick=True)
    sc.text("▲ one digit of the key changed: %s → %s" % (old_digit, new_digit), ("r", 1830), gy + 42, 30, ERR, MONO, t0=5.8, anim="fade")
    sc.text("static_assert(...) is evaluated by the compiler (CTFE), not at runtime", 80, 190 + h + 40, 34, ACC2, SANS, t0=2.5, anim="fade")
    sc.text("g2_is_valid: canonical coordinates, on the twist curve, in the order-r subgroup", 80, 190 + h + 100, 30, DIM, SANS, t0=3.2, anim="fade")
    sc.caption("Every point of the verifying key is checked by static_assert while compiling.", 1.0, 5.4)
    sc.caption("Change one digit and the verifier does not compile.", 6.4)

    # ---- 4b. the real error
    sc = new("s04b_error", 9.0, "Fe catches a bad key at compile time", "compile-time safety", 3)
    el = [l for l in bad_out.splitlines()]
    first = []
    for l in el:
        first.append(l)
        if l.startswith("error") and len(first) > 1 and sum(1 for x in first if x.startswith("error")) == 2:
            first.pop()
            break
    first = [l for l in first if l.strip()][:9]
    tail = [l for l in el if l.startswith("ERROR") or l.startswith("test result")]
    L = ["$ fe test"] + first + tail[:2]
    sc.term(L, 60, 190, 1800, 28, 41, t0=0.3, per=0.25, title="semaphore_verifier (one digit of BETA_X0 changed)", maxc=105)
    ai = next(i for i, l in enumerate(L) if "static assertion failed" in l)
    flash(sc, 70, 190 + 54 + ai * 41 - 3, 700, 40, 0.3 + ai * 0.25 + 1.5, ERR)
    sc.text("✗", 400, 826, 64, ERR, SYM, t0=4.8)
    sc.text("compile error: no contract is produced", 490, 830, 56, ERR, SANSB, t0=4.8)
    sc.caption("An invalid key does not compile: zero runtime gas for key checks.", 1.0)

    # ---- 5a. typed safety
    sc = new("s05a_typed", 10.0, "Typed safety: raw u256 is not a point", "typed safety", 4)
    bn = Src("src/ec/bn254.fe")
    g1_lines, g1_nums = join(bn.grab("pub struct G1 {", 1), bn.grab("pub fn from_coords(_ x: u256, _ y: u256) -> Option<G1>", 7))
    sc.code(g1_lines, 60, 190, 1070, 28, 40, t0=0.3, per=0.25, title="src/ec/bn254.fe", numbers=g1_nums)
    pe = [l for l in probe_out.splitlines() if l.strip() and l.strip() != "│"]  # blank gutter lines elided
    blk = []
    for l in pe:
        if l.startswith("error") and blk:
            break
        blk.append(l)
    blk = ["$ fe build   # G1 { x: 1, y: 2 }"] + blk[:5]
    sc.term(blk, 1140, 190, 720, 28, 40, t0=3.2, per=0.3, title="a caller outside fe_zk", maxc=40)
    sc.text("raw u256 cannot become a point", "c", 740, 70, ACC, SANSB, t0=5.8)
    sc.caption("G1::from_coords is the only way in: coordinates below q, on the curve, never reduced.", 0.8, 5.6)
    sc.caption("The fields are private, so there is no unchecked way to build a G1.", 5.9)

    # ---- 5b. G2Proof
    sc = new("s05b_g2", 9.0, "Typed safety: G2Proof", "typed safety", 4)
    # re-grab with the real signature line as the first line
    i0 = bn.find("-> Option<G2Proof> {")
    sig_i = i0
    body = bn.lines[sig_i:sig_i + 7]
    g2l = [bn.lines[bn.find("pub struct G2Proof")], "", bn.lines[bn.find("impl G2Proof {")]] + body
    g2n = [bn.find("pub struct G2Proof") + 1, 0, bn.find("impl G2Proof {") + 1] + list(range(sig_i + 1, sig_i + 8))
    k = bn.find("pub fn from_g2")
    g2l += ["", bn.lines[k], bn.lines[k + 1], bn.lines[k + 2], "}"]
    g2n += [0, k + 1, k + 2, k + 3, k + 4]
    pos, h = sc.code(g2l, 60, 190, 1800, 28, 36, t0=0.3, per=0.22, title="src/ec/bn254.fe", numbers=g2n)
    sx, sy = pos(0, 0)
    flash(sc, sx - 6, sy - 3, int(cw(28) * len(g2l[0])) + 12, 40, 2.8)
    sc.text("◀ no conversion to G2: it can only reach the pairing", sx + int(cw(28) * len(g2l[0])) + 30, sy, 28, ACC2, MONO, t0=3.2, anim="fade")
    sc.text("the subgroup check runs in the pairing precompile (EIP-197)", 80, 190 + h + 30, 34, TXT, SANS, t0=4.2, anim="fade")
    sc.caption("G2 points from a proof: canonical and on-twist checked in Fe, subgroup completed by the pairing.", 1.0)

    # ---- 6a. generic Field
    p2 = Src("src/hash/poseidon2.fe")
    sc = new("s06a_generic", 8.0, "One implementation, every field", "generic field layer", 5)
    fi = p2.find("pub fn permute_generic")
    gl = [p2.lines[fi - 1]] + p2.lines[fi:fi + 7]
    gn = list(range(fi, fi + 8))
    for pat in ("let half = NE", "s[0] = sbox(s[0].add(F::reduce"):
        j = p2.find(pat, fi)
        gl.append(p2.lines[j])
        gn.append(j + 1)
    gl = [l for l in gl]
    ded = min(len(l) - len(l.lstrip()) for l in gl if l.strip() and not l.startswith(("#", "pub", ")")))
    pos, h = sc.code(gl, 60, 190, 1800, 28, 42, t0=0.3, per=0.2, title="src/hash/poseidon2.fe: the single Poseidon2 permutation", numbers=gn)
    fx_, fy_ = pos(1, gl[1].index("F: Field"))
    flash(sc, fx_ - 6, fy_ - 3, int(cw(28) * len("F: Field + Copy")) + 12, 40, 3.6)
    sc.text("F: Field", 80, 190 + h + 36, 40, ACC, MONOB, t0=4.6, anim="fade")
    sc.text("is all the permutation body knows about the field", 80 + int(cw(40) * 9), 190 + h + 40, 34, TXT, SANS, t0=4.6, anim="fade")
    sc.caption("Hash, Merkle and transcript code is written once against the Field trait.", 1.0)

    # ---- 6b. two instantiations
    sc = new("s06b_inst", 9.0, "One implementation, every field", "generic field layer", 5)
    a_l, a_n = p2.grab("pub fn bn254_t3_permute_generic", 3, dedent=False)
    b_l, b_n = p2.grab("pub fn koalabear_t16_permute_generic", 0, dedent=False, until="}")
    ha = 54 + len(a_l) * 40 + 20
    sc.code(a_l, 60, 190, 1800, 28, 40, t0=0.3, per=0.25, title="BN254 scalar field · 254-bit · t = 3", numbers=a_n)
    sc.code(b_l, 60, 190 + ha + 24, 1800, 28, 40, t0=2.4, per=0.25, title="KoalaBear · 31-bit · t = 16", numbers=b_n)
    sc.caption("Same permute_generic body, two fields: monomorphised at compile time, zero runtime cost.", 1.0)

    # ---- 7. quickstart (three beats)
    q = Src("examples/quickstart/src/lib.fe")
    sc = new("s07_quick", 15.0, "Use it in your contract", "quickstart", 6)
    use_l, use_n = join(q.grab("use fe_zk::", 1), q.grab("use multiplier2_verifier", 1))
    hr_l, hr_n = q.grab("pub fn horner", 0, until="}")
    l1 = use_l + [""] + hr_l
    n1 = use_n + [0] + hr_n
    sc.code(l1, 60, 190, 1800, 28, 40, t0=0.3, per=0.2, title="examples/quickstart/src/lib.fe", numbers=n1, t1=5.4)
    sc.text("a helper written once, usable with any field", 80, 190 + 54 + len(l1) * 40 + 30, 34, ACC2, SANS, t0=2.5, t1=5.4, anim="fade")
    c_l, c_n = join(q.grab("Commit { value, salt } -> u256 {", 6),
                    q.grab("Check { a, b, c, input } -> bool", 0, until="        }"))
    sc.code(c_l, 60, 190, 1800, 28, 38, t0=5.9, per=0.2, title="examples/quickstart/src/lib.fe", numbers=c_n, t1=10.6)
    sc.text("static call to the generated verifier: it cannot touch our storage", 80, 190 + 54 + len(c_l) * 38 + 30, 34, ACC2, SANS, t0=8.0, t1=10.6, anim="fade")
    ql = [l for l in quick_out.splitlines() if l.strip()]
    tl = ["$ cd examples/quickstart && fe test"] + ql[-8:]
    sc.term(tl, 60, 190, 1800, 28, 42, t0=11.1, per=0.25, title="examples/quickstart")
    ri = next(i for i, l in enumerate(tl) if "test result" in l)
    flash(sc, 70, 190 + 54 + ri * 42 - 3, 640, 40, 11.1 + ri * 0.25 + 1.2, OK)
    sc.caption("Generated verifier, circomlib-compatible Poseidon and one generic helper in a %d-line contract." % len(q.lines), 11.4)

    # ---- 8a. Life on chain
    sc = new("s08a_life", 9.5, "zk Game of Life, on chain", "game of life", 7)
    rate = 9
    boards = [bits_to_rows(f["board"]) for f in play["frames"]]
    d, nfr, _ = life_clip(SRC / "play", boards, rate, hold_tail=1.5)
    i = sc.add_input(["-framerate", str(FPS), "-i", str(d / "f%04d.ppm")])
    sc.overlay(i, board_pre(704, 5, True, 18, "p"), 80, 190)
    N = len(play["frames"]) - 1
    sc.text("GENERATION", 900, 210, 26, DIM, MONO, t0=0.3, anim="fade", tick=True)
    sc.text("ALIVE CELLS", 900, 450, 26, DIM, MONO, t0=0.3, anim="fade", tick=True)
    sc.text("step() GAS", 900, 680, 26, DIM, MONO, t0=0.3, anim="fade", tick=True)
    for g, f in enumerate(play["frames"]):
        ta, tb = g / float(rate), (g + 1) / float(rate)
        last = g == len(play["frames"]) - 1
        t1 = None if last else tb
        alive = sum(map(sum, boards[g]))
        sc.text("%d" % g, 900, 250, 150, ACC, MONOB, t0=ta, t1=t1, anim="cut", tick=True)
        sc.text("/ %d" % N, 900 + int(cw(150) * 2) + 20, 330, 60, FAINT, MONOB, t0=ta, t1=t1, anim="cut", tick=True)
        sc.text("%d" % alive, 900, 490, 110, TXT, MONOB, t0=ta, t1=t1, anim="cut", tick=True)
        sc.text(str(f["step_gas"]) if f["step_gas"] else "-", 900, 720, 110, ACC2, MONOB, t0=ta, t1=t1, anim="cut", tick=True)
    sc.text("word-parallel bit tricks, no per-cell loops", 900, 850, 30, DIM, SANS, t0=1.0, anim="fade", tick=True)
    sc.caption("A 16x16 torus packed into one u256, stepped by the Fe contract: one call per generation.", 0.8)
    sc.track(0.3, 1)

    # ---- 8b. the proof
    gens = prove["generations"]
    sb, eb = bits_to_rows(prove["start_board"]), bits_to_rows(prove["end_board"])
    s_alive, e_alive = sum(map(sum, sb)), sum(map(sum, eb))
    sc = new("s08b_proof", 9.0, "%d generations, one proof" % gens, "game of life", 7)
    for k, (rows, x, t0) in enumerate(((sb, 130, 0.6), (eb, 1358, 3.4))):
        pp = SRC / ("prove_%d.ppm" % k)
        write_ppm(pp, rows)
        i = sc.add_input(["-loop", "1", "-framerate", str(FPS), "-t", "%.2f" % sc.dur, "-i", str(pp)])
        sc.overlay(i, board_pre(432, 3, True, 12, "q%d" % k) + ",format=rgba,fade=t=in:st=%.2f:d=0.5:alpha=1" % t0, x, 250)
    sc.text("start board", 130, 700, 30, DIM, MONO, t0=0.8, tick=True)
    sc.text("alive %d" % s_alive, 130, 740, 40, TXT, MONOB, t0=0.8, tick=True)
    sc.text("end board, generation %d" % gens, 1358, 700, 30, DIM, MONO, t0=3.6, tick=True)
    sc.text("alive %d" % e_alive, 1358, 740, 40, TXT, MONOB, t0=3.6, tick=True)
    sc.text("%d generations" % gens, "c", 320, 60, ACC, SANSB, t0=1.6)
    sc.text("one Groth16 proof", "c", 400, 60, TXT, SANSB, t0=2.2)
    sc.text("→", "c", 480, 120, ACC, SYM, t0=2.8, anim="left")
    sc.text("verified on chain", "c", 640, 40, DIM, SANS, t0=3.6, anim="fade")
    sc.caption("Poseidon commitments of the start and the end board are the proof's public inputs.", 1.0)

    # ---- 8c. gas bars + rejected
    sc = new("s08c_gas", 13.0, "Gas: stepping vs one proof", "game of life", 7)
    gv = lambda pat: int(re.search(pat + r"\s+(\d+)\s+(\d+)", prove_out).group(1))
    rows = [("%d × step()" % gens, gv(r"separate step\(\) calls"), ACC2), ("one run(board, %d)" % gens, gv(r"run\(board, \d+\) call"), ACC),
            ("one proof (proveRun)", gv(r"proveRun \(verify proof of the same run\)"), OK)]
    mx = max(r[1] for r in rows)
    bx, bw = 640, 940
    for k, (lab, v, col) in enumerate(rows):
        y = 210 + k * 105
        t0 = 0.6 + k * 1.1
        sc.text(lab, 80, y + 12, 38, TXT, SANSB, t0=t0)
        w = int(bw * v / mx)
        grow_bar(sc, bx, y, w, 70, col, t0 + 0.2, 0.2 + 1.8 * v / mx)
        sc.text(fmt(v), bx + w + 24, y + 8, 44, col, MONOB, t0=t0 + 0.4 + 1.8 * v / mx, anim="cut", tick=True)
        sc.track(t0, y)
    sc.text("execution gas, revm (life-cli --prove)", bx, 180, 24, DIM, MONO, t0=0.6, anim="fade", tick=True)
    rj = [l for l in prove_out.splitlines() if "REJECTED" in l]
    sc.term(["$ life-cli --prove"] + rj, 80, 560, 900, 30, 42, t0=5.8, per=0.9, title="negative cases", colour=lambda l: OK if "REJECTED" in l else TXT, maxc=48)
    be = next(l for l in prove_out.splitlines() if l.startswith("break-even"))
    be = be.replace("break-even: ", "")
    wr = textwrap.wrap(be, 38)
    for k, l in enumerate(wr):
        sc.text(l, 1040, 600 + k * 44, 34, TXT if k == 0 else DIM, SANS, t0=10.0 + k * 0.2, anim="fade", tick=True)
    sc.text("break-even", 1040, 560, 26, ACC, MONOB, t0=10.0, anim="fade", tick=True)
    sc.track(5.8, 600)
    sc.caption("A tampered proof, a wrong end board and a wrong start board are all rejected.", 6.0)

    # ---- 9. every family
    fam = families()
    sc = new("s09_families", 14.0, "Every proof family", "coverage", 8)
    hy = 196
    sc.text("FAMILY", 100, hy, 24, DIM, MONO, t0=0.3, anim="fade", tick=True)
    sc.text("FE EXEC GAS", ("r", 890), hy, 24, DIM, MONO, t0=0.3, anim="fade", tick=True)
    sc.text("FE / SOLIDITY (tick = 1.0x)", 960, hy, 24, DIM, MONO, t0=0.3, anim="fade", tick=True)
    sc.text("CODE SIZE", 1440, hy, 24, DIM, MONO, t0=0.3, anim="fade", tick=True)
    one_x, scale = 960, 190
    bls_n = len(re.findall(r"#\[test", (ROOT / "src/tests/ec_bls.fe").read_text()))
    rows9 = [(n, d) for n, d in fam]
    for k, (name, d) in enumerate(rows9 + [("BLS12-381 (EIP-2537)", None), ("EIP-4844 point evaluation", None)]):
        y = hy + 44 + k * 80
        t0 = 0.8 + k * 1.2
        sc.box(80, y - 6, 1760, 70, PANEL2 + "@0.55", t0)
        sc.text(name, 100, y + 8, 32, TXT, SANSB, t0=t0)
        if d:
            sc.text(fmt(d["gas"]), ("r", 890), y + 10, 32, ACC2, MONOB, t0=t0 + 0.15, tick=True)
            if d["hi"]:
                lab = ("%.2fx" % d["hi"]) if d["lo"] == d["hi"] else ("%.2f-%.2fx" % (d["lo"], d["hi"]))
                grow_bar(sc, one_x, y + 6, int(scale * d["hi"]), 40, ACC, t0 + 0.2, 0.8, step=3)
                sc.box(one_x + scale, y, 3, 52, TXT, t0 + 0.2, fade=False)
                sc.text(lab, one_x + int(scale * d["hi"]) + 16, y + 10, 30, TXT, MONOB, t0=t0 + 1.0, anim="cut", tick=True)
            else:
                sc.text("no Solidity reference exists", one_x, y + 14, 26, DIM, MONO, t0=t0 + 0.3, tick=True)
            ok = d["code"] <= 24576
            sc.text("%s B" % fmt(d["code"]), 1440, y + 10, 30, TXT, MONOB, t0=t0 + 0.3, tick=True)
            sc.text("EIP-170 OK" if ok else "over EIP-170", 1660, y + 12, 26, "0x04120a" if ok else "0xffffff", MONOB, t0=t0 + 0.5, anim="cut", tick=True,
                    box=((OK if ok else ERR) + "@0.95", 8))
        else:
            ref = "py_ecc 8.0.0 vectors" if name.startswith("BLS") else "c-kzg-4844 test vector"
            sc.text("wrapper tested against %s" % ref, one_x - 840 + 840, y + 14, 26, DIM, MONO, t0=t0 + 0.3, tick=True)
        sc.track(t0, y)
    sc.caption("Fe / Solidity execution gas on the same proof, same revm (docs/GAS.md).", 1.0)

    # ---- 9b. anonymous apps (v0.2.0)
    sc = new("s09b_anon", 14.0, "Anonymous apps in Fe", "anon module", 9)
    li = Src("src/anon/lean_imt.fe")
    nu = Src("src/anon/nullifier.fe")
    a, an = li.grab("pub fn insert(mut self", 1)
    b, bn_ = li.grab("pub fn is_known_root", 1)
    c, cn = nu.grab("pub fn spend(mut self", 8)
    ls, ns = join((a, an), (b, bn_))
    pos, h1 = sc.code(ls, 60, 190, 1800, 28, 38, t0=0.3, per=0.3, title="src/anon/lean_imt.fe: LeanIMT, root-compatible with Semaphore v4", numbers=ns)
    pos, h2 = sc.code(c, 60, 190 + h1 + 30, 1800, 28, 38, t0=2.0, per=0.2, title="src/anon/nullifier.fe: one use per scope", numbers=cn)
    ty = 190 + h1 + h2 + 80
    chk = ("anon_vote", "anonymous voting (Semaphore v4 ingot, nullifier per poll)"), ("tornado", "Tornado withdraw verifier: real mainnet proofs")
    for k, (key, lab) in enumerate(chk):
        r = next((o for l, rc, o, p_, f in results if l.endswith(key) or l.startswith("examples/" + key)), None)
        m = re.findall(r"test result: \w+\. (\d+) passed", r or "")
        t0 = 5.0 + k * 1.5
        sc.text("✓", 80, ty + k * 70, 48, OK, SYM, t0=t0, anim="fade")
        sc.text(lab, 160, ty + k * 70 + 4, 34, TXT, SANSB, t0=t0)
        sc.text("%s tests passed" % (m[0] if m else "?"), ("r", 1840), ty + k * 70 + 8, 28, DIM, MONO, t0=t0 + 0.4, anim="fade")
    sc.caption("v0.2.0: LeanIMT, scoped nullifiers, Semaphore v4 ingot, voting, Tornado verifier.", 1.0)

    # ---- 10. tested
    td = testing_totals()
    sc = new("s10_tested", 11.0, "Tested like it matters", "verification", 10)
    cards = [(n_tests, "Fe tests passing", "fe test, %d ingots, 0 failed" % len(results)),
             (td["cases"], "differential cases", "Fe vs reference verifiers: %d bugs" % td["bugs"]),
             (safety_rules(), "safety rules", "enforcement point + test, each")]
    for k, (v, lab, sub) in enumerate(cards):
        x, t0 = 80 + k * 600, 0.4 + k * 0.5
        sc.box(x, 230, 560, 330, PANEL2 + "@0.92", t0)
        sc.box(x, 230, 560, 330, LINE, t0, fill=False, thick=2)
        sc.box(x, 230, 560, 6, ACC, t0, fade=False)
        counter(sc, v, x + 30, 280, 112, ACC, t0 + 0.3, 2.0)
        sc.text(lab, x + 30, 430, 40, TXT, SANSB, t0=t0 + 0.4, tick=True)
        sc.text(sub, x + 30, 490, 24, DIM, MONO, t0=t0 + 0.5, tick=True)
        sc.track(t0, 230)
    sem_pass = [l for l in sem_out.splitlines() if "test result" in l][-1]
    sc.text("✓", 80, 640, 80, OK, SYM, t0=4.2, anim="fade")
    sc.text("Semaphore v4: real ceremony key verified", 180, 650, 60, TXT, SANSB, t0=4.2)
    sc.text("%s: %s" % (sem_label, sem_pass), 180, 740, 28, DIM, MONO, t0=4.8, anim="fade")
    sc.caption("Every verifier test: a valid proof accepted; tampered, non-canonical, off-curve and swapped inputs rejected.", 6.0)

    # ---- 11. outro
    sc = new("s11_outro", 7.0)
    sc.text("fe-zk", "c", 250, 190, ACC, SANSB, t0=0.3)
    sc.text("github.com/0xitsss/fe-zk", "c", 480, 68, TXT, SANSB, t0=1.0)
    sc.text("%s  ·  Apache-2.0  ·  docs/USAGE.md" % fe_ver, "c", 600, 40, DIM, MONO, t0=1.6, anim="fade")
    sc.text("Verifiers, generator, safety rules and tests: all in Fe", "c", 720, 40, ACC2, SANS, t0=2.2, anim="fade")
    sc.track(0.3, 1)

    # ------------------------------------------------------------ render
    for s in scenes:
        s.check()
    log("render %d scenes" % len(scenes))
    sel = [s for s in scenes if not only or s.name in only]
    with ThreadPoolExecutor(6) as ex:
        list(ex.map(lambda s: (log("  ", s.name), s.render(CLIPS))[1], sel))
    if only:
        log("VIDEO_ONLY set: skipping assembly")
        return
    clips = [CLIPS / (s.name + ".mp4") for s in scenes]
    durs = [s.dur for s in scenes]
    kinds = ["fade", "slideleft", "fade", "wipeleft", "fadeblack", "smoothleft", "fade", "slideup", "fade", "smoothleft", "fade",
             "slideleft", "fade", "fadeblack", "smoothup", "fade", "fade"]
    final = VIDEO / "fe-zk-demo.mp4"
    total = assemble(clips, durs, final, 0.55, kinds)
    log("final %.1f s" % total)

    # teaser GIF: 12 s from the Life game scene
    gif = VIDEO / "teaser.gif"
    off = 0.0
    names = [s.name for s in scenes]
    ts = 0.0
    for s in scenes:
        if s.name == "s08a_life":
            break
        ts += s.dur - 0.55
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-ss", "%.2f" % (ts + 0.3), "-t", "12", "-i", str(final), "-vf",
                    "fps=12,scale=800:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96[p];[b][p]paletteuse=dither=bayer:bayer_scale=4",
                    str(gif)], check=True)
    # contact sheet: one frame per scene (mid scene)
    cdir = TMP / "contact"
    cdir.mkdir()
    t = 0.0
    for k, s in enumerate(scenes):
        mid = t + s.dur * (0.38 if k == 0 else 0.62)
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-ss", "%.2f" % mid, "-i", str(final), "-frames:v", "1",
                        "-vf", "scale=640:-1", str(cdir / ("c%02d.png" % k))], check=True)
        t += s.dur - 0.55
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", "1", "-i", str(cdir / "c%02d.png"), "-vf",
                    "tile=4x5:padding=8:color=0x0b0f14", "-frames:v", "1", str(VIDEO / "contact.png")], check=True)
    log("done in %.0f s: %s (%.1f s), %s" % (time.time() - t_start, final, total, gif))
    (TMP / "scene_times.json").write_text(json.dumps([(s.name, s.dur) for s in scenes]))


if __name__ == "__main__":
    main()
