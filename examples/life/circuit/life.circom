pragma circom 2.0.0;

include "circomlib/circuits/poseidon.circom";

// zk Game of Life: N generations of B3/S23 on a 16x16 torus.
//
// Board layout is identical to examples/life (Fe): bit index = row*16 + col, col 0 is the least
// significant bit of the 16-bit row lane. A board packs into lo = bits 0..127, hi = bits 128..255.
//
// Public signals, in snarkjs order (public inputs in declaration order; no outputs):
//   [0] startCommit = Poseidon(lo, hi) of the start board
//   [1] endCommit   = Poseidon(lo, hi) of the board after N generations
// N is fixed by the circuit (hence by the verifying key). The start board is private.
//
// One generation costs 7 constraints per cell (1792 per generation) when compiled with --O2,
// which substitutes away every purely linear constraint (neighbour sums, b0, out, packing):
//   n = 8-neighbour sum (linear); n = b0 + 2 b1 + 4 b2 + 8 b3 with 4 boolean bits (4 constraints;
//   the equality itself is linear and free); nor = 1 - b2 - b3 (b2 and b3 are never both 1
//   because n <= 8, so this is the boolean "n < 4"); p = b1 * nor (n in {2,3}, 1 constraint);
//   q = p * b0 (n == 3, 1 constraint); next = q + alive * (p - q) (n == 3, or n == 2 and
//   alive, 1 constraint).
// Build: circom --O2 (the cost claim above depends on it).

template Gen() {
    signal input in[256];
    signal output out[256];

    signal b[256][4];
    signal p[256];
    signal q[256];
    signal t[256];

    for (var r = 0; r < 16; r++) {
        for (var c = 0; c < 16; c++) {
            var i = r * 16 + c;
            var n = 0;
            for (var dr = 0; dr < 3; dr++) {
                for (var dc = 0; dc < 3; dc++) {
                    if (dr != 1 || dc != 1) {
                        n += in[((r + dr + 15) % 16) * 16 + ((c + dc + 15) % 16)];
                    }
                }
            }
            // witness: bit decomposition of n
            b[i][0] <-- n & 1;
            b[i][1] <-- (n >> 1) & 1;
            b[i][2] <-- (n >> 2) & 1;
            b[i][3] <-- (n >> 3) & 1;
            for (var k = 0; k < 4; k++) {
                b[i][k] * (b[i][k] - 1) === 0;
            }
            b[i][0] + 2 * b[i][1] + 4 * b[i][2] + 8 * b[i][3] === n;
            p[i] <== b[i][1] * (1 - b[i][2] - b[i][3]);
            q[i] <== p[i] * b[i][0];
            t[i] <== in[i] * (p[i] - q[i]);
            out[i] <== q[i] + t[i];
        }
    }
}

// Packs 256 boolean cells into (lo, hi) 128-bit halves; bit i has weight 2^(i mod 128).
template Pack() {
    signal input cells[256];
    signal output lo;
    signal output hi;
    var l = 0;
    var h = 0;
    var w = 1;
    for (var i = 0; i < 128; i++) {
        l += cells[i] * w;
        h += cells[128 + i] * w;
        w = w + w;
    }
    lo <== l;
    hi <== h;
}

template Life(N) {
    signal input startCommit;
    signal input endCommit;
    signal input start[256]; // private

    // start cells must be boolean (later cells are boolean by construction)
    for (var i = 0; i < 256; i++) {
        start[i] * (start[i] - 1) === 0;
    }

    component packStart = Pack();
    component packEnd = Pack();
    component hs = Poseidon(2);
    component he = Poseidon(2);
    for (var i = 0; i < 256; i++) packStart.cells[i] <== start[i];
    hs.inputs[0] <== packStart.lo;
    hs.inputs[1] <== packStart.hi;
    hs.out === startCommit;

    component gen[N];
    for (var g = 0; g < N; g++) {
        gen[g] = Gen();
        for (var i = 0; i < 256; i++) {
            if (g == 0) {
                gen[g].in[i] <== start[i];
            } else {
                gen[g].in[i] <== gen[g - 1].out[i];
            }
        }
    }
    for (var i = 0; i < 256; i++) packEnd.cells[i] <== gen[N - 1].out[i];
    he.inputs[0] <== packEnd.lo;
    he.inputs[1] <== packEnd.hi;
    he.out === endCommit;
}
