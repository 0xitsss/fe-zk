#!/usr/bin/env python3
"""Turn life-cli frame dumps into a self-contained animated SVG (CSS keyframes, no scripts).

usage: python3 -I scripts/make-life-svg.py PLAY.json PROVE.json OUT.svg   (scripts/make-life-svg.sh does it all)

PLAY.json  from `life-cli --pattern P --gens N --no-anim --frames-json PLAY.json`
PROVE.json from `life-cli --prove --frames-json PROVE.json`
Every board and every number comes from these files (boards are the u256 values returned by
the Fe `Life` contract in revm); nothing is hard-coded here except layout and colours.
GitHub renders README SVGs through <img>: scripts are stripped, CSS animations still run.
"""
import json
import sys

W, H = 640, 420
BG, DEAD, ALIVE, FG, DIM = "#0d1117", "#1b222c", "#3fb950", "#e6edf3", "#9aa7b4"
BAR_STEPS, BAR_RUN, BAR_PROOF = "#8b949e", "#58a6ff", "#3fb950"
DT = 0.12          # seconds per generation
HOLD = 5.0         # seconds the final scene stays
N = 16


def cells(board_hex):
    b = int(board_hex, 16)
    return [(i // N, i % N) for i in range(N * N) if (b >> i) & 1]


def grid_path(board_hex, x0, y0, size):
    """One <path> for all alive cells (bit index = row * 16 + col, as in the contract)."""
    d = "".join(f"M{x0 + c * size} {y0 + r * size}h{size - 1}v{size - 1}h-{size - 1}z" for r, c in cells(board_hex))
    return f'<path fill="{ALIVE}" d="{d}"/>'


def dead_grid(x0, y0, size):
    return f'<rect x="{x0 - 1}" y="{y0 - 1}" width="{N * size + 1}" height="{N * size + 1}" rx="3" fill="{DEAD}"/>'


def fmt(n):
    return f"{int(n):,}"


def text(x, y, s, size=13, fill=FG, anchor="start", weight="normal"):
    s = s.replace("&", "&amp;").replace("<", "&lt;")
    return (f'<text x="{x}" y="{y}" font-size="{size}" fill="{fill}" text-anchor="{anchor}" '
            f'font-weight="{weight}">{s}</text>')


def main(play_path, prove_path, out_path):
    play = json.load(open(play_path))
    prove = json.load(open(prove_path))
    assert play["mode"] == "play" and prove["mode"] == "prove"
    frames = play["frames"]
    n_play = len(frames)
    total = n_play * DT + HOLD
    win = DT / total * 100
    win_final = HOLD / total * 100
    cs = 18
    gx, gy = (W - N * cs) // 2, 52

    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" '
           f'font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace">',
           "<title>fe-zk zk Game of Life: frames from the compiled Fe contract executed in revm</title>"]
    out.append("<style>")
    out.append(".f{opacity:0;animation:f %.3fs linear infinite}" % total)
    out.append("@keyframes f{0%%{opacity:1}%.4f%%{opacity:1}%.4f%%{opacity:0}100%%{opacity:0}}" % (win, win + 0.0001))
    out.append(".z{opacity:0;animation:z %.3fs linear infinite}" % total)
    out.append("@keyframes z{0%%{opacity:1}%.4f%%{opacity:1}%.4f%%{opacity:0}100%%{opacity:0}}" % (win_final, win_final + 0.0001))
    wq = n_play * DT / total * 100
    out.append(".q{opacity:0;animation:q %.3fs linear infinite}" % total)
    out.append("@keyframes q{0%%{opacity:1}%.4f%%{opacity:1}%.4f%%{opacity:0}100%%{opacity:0}}" % (wq, wq + 0.0001))
    out.append("</style>")
    out.append(f'<rect width="{W}" height="{H}" rx="10" fill="{BG}"/>')
    out.append(text(W // 2, 30, "fe-zk  |  zk Game of Life, 16x16 torus in one u256", 15, FG, "middle", "bold"))

    # scene 1: play, one group per generation
    out.append(dead_grid(gx, gy, cs).replace("<rect", '<rect class="q"'))
    for fr in frames:
        g = fr["generation"]
        if fr["step_gas"] is None:
            cap = f'generation {g} · start: {play["pattern"]}'
        else:
            cap = f'generation {g} · step gas {fmt(fr["step_gas"])} (Fe contract, revm)'
        out.append(f'<g class="f" style="animation-delay:{g * DT:.2f}s">'
                   + grid_path(fr["board"], gx, gy, cs) + text(W // 2, 392, cap, 14, FG, "middle") + "</g>")
    out.append(text(W // 2, 410, "Fe contract steps the whole board with word-parallel bit operations", 11, DIM, "middle"))

    # scene 2: proof vs on-chain stepping, numbers from PROVE.json
    n = prove["generations"]
    d = f'{n_play * DT:.2f}s'
    s2 = [f'<g class="z" style="animation-delay:{d}">']
    gsz = 9
    sx, ex, sy = 40, W - 40 - N * gsz, 56
    for x in (sx, ex):
        s2.append(dead_grid(x, sy, gsz))
    s2.append(grid_path(prove["start_board"], sx, sy, gsz))
    s2.append(grid_path(prove["end_board"], ex, sy, gsz))
    s2.append(text(sx + N * gsz // 2, sy + N * gsz + 16, "start board", 12, DIM, "middle"))
    s2.append(text(ex + N * gsz // 2, sy + N * gsz + 16, f"after {n} generations", 12, DIM, "middle"))
    s2.append(text(W // 2, sy + 62, f"{n} generations", 15, FG, "middle", "bold"))
    s2.append(text(W // 2, sy + 84, "one Groth16 proof", 15, ALIVE, "middle", "bold"))
    bars = [(f"{n} x step()", prove["steps_gas"], BAR_STEPS),
            (f"run(board, {n})", prove["run_gas"], BAR_RUN),
            ("verify the proof", prove["verify_gas"], BAR_PROOF)]
    mx = max(v for _, v, _ in bars)
    bx, bw = 190, 300
    for i, (label, v, col) in enumerate(bars):
        y = 238 + i * 38
        s2.append(text(bx - 10, y + 15, label, 13, FG, "end"))
        s2.append(f'<rect x="{bx}" y="{y}" width="{bw * v / mx:.1f}" height="22" rx="3" fill="{col}"/>')
        s2.append(text(bx + bw * v / mx + 8, y + 16, f"{fmt(v)} gas", 13, FG))
    s2.append(text(W // 2, 392, f'{n} generations, one Groth16 proof: verify {fmt(prove["verify_gas"])} gas '
                   f'vs run() {fmt(prove["run_gas"])} gas', 13, FG, "middle"))
    s2.append(text(W // 2, 410, "verifier generated by fe-zk-gen, executed in revm; same start and end board", 11, DIM, "middle"))
    s2.append("</g>")
    out += s2
    out.append("</svg>")
    svg = "\n".join(out) + "\n"
    open(out_path, "w").write(svg)
    print(f"{out_path}: {len(svg)} bytes, {n_play} frames + final scene, loop {total:.1f}s")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
