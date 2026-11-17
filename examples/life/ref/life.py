#!/usr/bin/env python3
"""Per-cell reference Game of Life (B3/S23) on a 16x16 torus.

Independent of the Fe bit tricks: cells are a list of lists, packed to an
integer only at the end. Bit index = row*16 + col (bit 0 = row 0, col 0).
Run `python3 -I life.py` to print vectors, `python3 -I life.py --write` to
regenerate vectors.json.
"""
import json
import os
import random
import sys

N = 16


def step(g):
    out = [[0] * N for _ in range(N)]
    for r in range(N):
        for c in range(N):
            n = sum(
                g[(r + dr) % N][(c + dc) % N]
                for dr in (-1, 0, 1)
                for dc in (-1, 0, 1)
                if (dr, dc) != (0, 0)
            )
            out[r][c] = 1 if n == 3 or (g[r][c] and n == 2) else 0
    return out


def from_cells(cells):
    g = [[0] * N for _ in range(N)]
    for r, c in cells:
        g[r % N][c % N] = 1
    return g


def pack(g):
    return sum(g[r][c] << (r * N + c) for r in range(N) for c in range(N))


def run(g, n):
    for _ in range(n):
        g = step(g)
    return g


def vectors():
    v = {}

    def add(name, g, ns):
        v[name] = {
            "start": pack(g),
            "steps": {str(n): pack(run(g, n)) for n in ns},
        }

    glider = [(0, 1), (1, 2), (2, 0), (2, 1), (2, 2)]
    add("glider", from_cells(glider), [1, 4])
    # glider moves by (+1, +1) every 4 generations
    assert pack(run(from_cells(glider), 4)) == pack(
        from_cells([(r + 1, c + 1) for r, c in glider])
    )
    add("blinker", from_cells([(5, 4), (5, 5), (5, 6)]), [1, 2])
    add("block", from_cells([(7, 7), (7, 8), (8, 7), (8, 8)]), [1, 5])
    # wrap-around: blinkers across the left/right and top/bottom seams
    add("blinker_wrap_h", from_cells([(3, 15), (3, 0), (3, 1)]), [1, 2])
    add("blinker_wrap_v", from_cells([(15, 9), (0, 9), (1, 9)]), [1, 2])
    # block across all four corners
    add("block_corner", from_cells([(15, 15), (15, 0), (0, 15), (0, 0)]), [1, 3])
    # glider that crosses the corner seams
    add("glider_corner", from_cells([(r + 13, c + 13) for r, c in glider]), [1, 8, 20])
    rnd = random.Random(20261117)
    board = [[rnd.getrandbits(1) for _ in range(N)] for _ in range(N)]
    add("random", board, [1, 10, 64, 256])
    add("full", [[1] * N for _ in range(N)], [1, 2])
    add("empty", [[0] * N for _ in range(N)], [1])
    return v


if __name__ == "__main__":
    v = vectors()
    if "--write" in sys.argv:
        p = os.path.join(os.path.dirname(os.path.abspath(__file__)), "vectors.json")
        with open(p, "w") as f:
            json.dump(v, f, indent=1)
            f.write("\n")
    for k, x in v.items():
        print(k, hex(x["start"]), {n: hex(y) for n, y in x["steps"].items()})
