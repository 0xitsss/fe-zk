#!/usr/bin/env python3
"""Emit src/hash/poseidon2_unrolled.fe: Poseidon2 for BN254 t=3 and KoalaBear t=16 with the
state in scalar locals and lazy modular reduction.

Usage: python3 scripts/gen-poseidon2-unrolled.py > src/hash/poseidon2_unrolled.fe

`poseidon2::permute_generic` stays the one-implementation-for-every-field version; this file
only reschedules the same arithmetic. `src/tests/hash.fe` checks both give identical outputs.
"""

T16 = 16


def ext16(ind="        "):
    """External layer on s0..s15 (Plonky3 mat4 per block, then lane sums), plain adds only."""
    o = []
    for b in range(4):
        a, bb, c, d = (f"s{4 * b + k}" for k in range(4))
        p = f"m{b}"
        o.append(f"{ind}let {p}_t01 = {a} + {bb}")
        o.append(f"{ind}let {p}_t23 = {c} + {d}")
        o.append(f"{ind}let {p}_t0123 = {p}_t01 + {p}_t23")
        o.append(f"{ind}let {p}_t01123 = {p}_t0123 + {bb}")
        o.append(f"{ind}let {p}_t01233 = {p}_t0123 + {d}")
        o.append(f"{ind}let {p}_o3 = {p}_t01233 + {a} + {a}")
        o.append(f"{ind}let {p}_o1 = {p}_t01123 + {c} + {c}")
        o.append(f"{ind}let {p}_o0 = {p}_t01123 + {p}_t01")
        o.append(f"{ind}let {p}_o2 = {p}_t01233 + {p}_t23")
    for l in range(4):
        o.append(f"{ind}let st{l} = m0_o{l} + m1_o{l} + m2_o{l} + m3_o{l}")
    for b in range(4):
        for l in range(4):
            o.append(f"{ind}s{4 * b + l} = m{b}_o{l} + st{l}")
    return o


def koala():
    o = []
    o.append("""/// KoalaBear t = 16 Poseidon2 (Plonky3 `default_koalabear_poseidon2_16`), x^3.
///
/// Bounds (all lanes are plain u256 sums/products of values below 2^40, no overflow):
///  - inputs are canonical: lane < P < 2^31.
///  - external layer: each mat4 output is a combination with coefficient sum 7, then each lane
///    adds the sum of 4 such outputs: lane <= 7B + 4 * 7B = 35 * B for lanes <= B.
///    With B = P this is <= 35 * P < 2^37.
///  - full round: (lane + rc) <= 36 * P < 2^37, cube < 2^111, `% P` brings it below P; the
///    following external layer gives <= 35 * P again. So the invariant "lane <= 35 * P on
///    entry to a full round" holds from the first round to the last.
///  - partial rounds need lanes < P: reduce once on entry. Then
///    sum of 16 lanes < 16 * P and `D[i] * lane < P * P < 2^62`, so
///    `(sum + D[i] * lane) % P` is exact and again < P.
///  - output: lanes <= 35 * P, reduced once.
pub fn permute_koalabear_t16(_ input: [u256; 16]) -> [u256; 16] {""")
    for i in range(16):
        o.append(f"    var s{i}: u256 = input[{i}]")
    o += ext16("    ")
    o.append("    var r: usize = 0")
    o.append("    while r < 4 {")
    for i in range(16):
        o.append(f"        let x{i} = s{i} + P2_KOALABEAR_T16_EXT[r * 16 + {i}]")
        o.append(f"        s{i} = (x{i} * x{i} * x{i}) % P")
    o += ext16()
    o.append("        r += 1")
    o.append("    }")
    for i in range(16):
        o.append(f"    s{i} = s{i} % P")
    o.append("    r = 0")
    o.append("    while r < 20 {")
    o.append("        let x = s0 + P2_KOALABEAR_T16_INT[r]")
    o.append("        s0 = (x * x * x) % P")
    o.append("        let sum = " + " + ".join(f"s{i}" for i in range(16)))
    for i in range(16):
        o.append(f"        s{i} = (sum + P2_KOALABEAR_T16_DIAG[{i}] * s{i}) % P")
    o.append("        r += 1")
    o.append("    }")
    o.append("    r = 0")
    o.append("    while r < 4 {")
    for i in range(16):
        o.append(f"        let x{i} = s{i} + P2_KOALABEAR_T16_EXT[64 + r * 16 + {i}]")
        o.append(f"        s{i} = (x{i} * x{i} * x{i}) % P")
    o += ext16()
    o.append("        r += 1")
    o.append("    }")
    o.append("    [" + ", ".join(f"s{i} % P" for i in range(16)) + "]")
    o.append("}")
    return "\n".join(o)


def bn254():
    E, I, D = "P2_BN254_T3_EXT", "P2_BN254_T3_INT", "P2_BN254_T3_DIAG"

    def ext(ind):
        return [
            f"{ind}let sum = s0 + s1 + s2",
            f"{ind}s0 = addmod(s0, sum, R)",
            f"{ind}s1 = addmod(s1, sum, R)",
            f"{ind}s2 = addmod(s2, sum, R)",
        ]

    def full(off):
        o = []
        for i in range(3):
            o.append(f"        s{i} = sbox5(addmod(s{i}, {E}[{off} + r * 3 + {i}], R))")
        return o + ext("        ")

    o = []
    o.append("""/// BN254 Fr t = 3 Poseidon2 (HorizenLabs `poseidon2_instance_bn256`), x^5.
///
/// Bounds: every addmod/mulmod result is < R, R < 0.1889 * 2^256. `sum` adds three lanes
/// (< 3 * R, no overflow) and goes straight into `addmod`, which is exact for any u256
/// operands, so no separate reduction is needed. Inputs are canonical (typed `Bn254Fr`).
pub fn permute_bn254_t3(_ a: u256, _ b: u256, _ c: u256) -> [u256; 3] {""")
    o.append("    var s0: u256 = a\n    var s1: u256 = b\n    var s2: u256 = c")
    o += ext("    ")
    o.append("    var r: usize = 0")
    o.append("    while r < 4 {")
    o += full(0)
    o.append("        r += 1\n    }")
    o.append("    r = 0")
    o.append("    while r < 56 {")
    o.append(f"        s0 = sbox5(addmod(s0, {I}[r], R))")
    o.append("        let sum = s0 + s1 + s2")
    for i in range(3):
        o.append(f"        s{i} = addmod(sum, mulmod({D}[{i}], s{i}, R), R)")
    o.append("        r += 1\n    }")
    o.append("    r = 0")
    o.append("    while r < 4 {")
    o += full(12)
    o.append("        r += 1\n    }")
    o.append("    [s0, s1, s2]\n}")
    return "\n".join(o)


print("""//! GENERATED FILE, do not edit: `scripts/gen-poseidon2-unrolled.py`.
//!
//! Poseidon2 for BN254 t = 3 and KoalaBear t = 16 on scalar locals with lazy reduction.
//! Same arithmetic as `poseidon2::permute_generic` (the single generic implementation, kept
//! as the reference); the unit tests assert both agree. Table entries are canonical (tests
//! `poseidon2_tables_are_canonical`). Arithmetic is `unchecked` only where the bound comment
//! on each function proves no overflow.
use std::evm::crypto::{addmod, mulmod}
use super::poseidon2_constants::{
    P2_BN254_T3_DIAG, P2_BN254_T3_EXT, P2_BN254_T3_INT,
    P2_KOALABEAR_T16_DIAG, P2_KOALABEAR_T16_EXT, P2_KOALABEAR_T16_INT,
}

const R: u256 = 21888242871839275222246405745257275088548364400416034343698204186575808495617
const P: u256 = 0x7f000001

#[arithmetic(unchecked)]
fn sbox5(_ x: u256) -> u256 {
    let x2 = mulmod(x, x, R)
    let x4 = mulmod(x2, x2, R)
    mulmod(x4, x, R)
}
""")
print("#[arithmetic(unchecked)]")
print(bn254())
print()
print("#[arithmetic(unchecked)]")
print(koala())
