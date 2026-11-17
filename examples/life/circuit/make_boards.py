#!/usr/bin/env python3
"""Prints `<start_hex> <end_hex>` for a pattern after N generations, using the per-cell
reference in ../ref/life.py (independent of the circuit and of the Fe bit tricks).
usage: python3 -I make_boards.py <pattern> <N> [seed]   patterns: glider rpentomino blinker random
"""
import os
import random
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "ref"))
import life  # noqa: E402

PATTERNS = {
    "glider": [(0, 1), (1, 2), (2, 0), (2, 1), (2, 2)],
    "blinker": [(5, 4), (5, 5), (5, 6)],
    # same cells and offset as life-cli: (0,1),(0,2),(1,0),(1,1),(2,1) shifted by (7,7)
    "rpentomino": [(7, 8), (7, 9), (8, 7), (8, 8), (9, 8)],
}

pattern, n = sys.argv[1], int(sys.argv[2])
seed = int(sys.argv[3]) if len(sys.argv) > 3 else 1
if pattern == "random":
    rnd = random.Random(seed)
    g = [[rnd.getrandbits(1) for _ in range(16)] for _ in range(16)]
elif pattern in PATTERNS:
    g = life.from_cells(PATTERNS[pattern])
else:
    sys.exit("unknown pattern " + pattern)
print(hex(life.pack(g)), hex(life.pack(life.run(g, n))))
