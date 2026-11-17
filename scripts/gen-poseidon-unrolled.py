#!/usr/bin/env python3
"""Emit src/hash/poseidon_unrolled.fe: the circomlib Poseidon permutation for t = 2..6 with the
state held in scalar locals (no array traffic) and lazy modular reduction.

Usage: python3 scripts/gen-poseidon-unrolled.py > src/hash/poseidon_unrolled.fe

Round structure, the constants layout and the parameters are those of `src/hash/poseidon.fe`
(`permute_array`, the readable reference); this file only changes how the same arithmetic is
scheduled. `src/tests/hash.fe` checks both give identical outputs.
"""

PARAMS = {2: 56, 3: 57, 4: 56, 5: 60, 6: 60}


def dot(terms):
    """Sum of mulmod results (each < R), reduced once. Bound: R < 0.1889 * 2^256, so 5 * R < 2^256."""
    n = len(terms)
    if n <= 5:
        return "(" + " + ".join(terms) + ") % R"
    # n == 6: two groups of three (< 3R each); addmod is exact for any u256 operands.
    return "addmod(" + " + ".join(terms[:3]) + ", " + " + ".join(terms[3:]) + ", R)"


def full_layer(T, c_off, mat):
    """sbox + round constants, then the matrix product, on locals s0..s{T-1}."""
    out = []
    for i in range(T):
        out.append(f"        let a{i} = addmod(sbox(s{i}), C[{c_off} + {i}], R)")
    for i in range(T):
        terms = [f"mulmod({mat}[{j * T + i}], a{j}, R)" for j in range(T)]
        out.append(f"        s{i} = {dot(terms)}")
    return out


def gen(T):
    n_p = PARAMS[T]
    C, S, M, P = (f"POSEIDON_{x}{T}" for x in "CSMP")
    ins = ", ".join(f"_ i{k}: u256" for k in range(1, T))
    o = []
    o.append(f"/// circomlib Poseidon, t = {T} ({T - 1} inputs). Inputs must be canonical (< R).")
    o.append(f"pub fn permute_t{T}({ins}) -> u256 {{")
    o.append(f"    var s0: u256 = C[0]")
    for i in range(1, T):
        o.append(f"    var s{i}: u256 = addmod(i{i}, C[{i}], R)")
    o.append("    var r: usize = 0")
    o.append("    while r < 3 {")
    o.append(f"        let off = (r + 1) * {T}")
    o += [l.replace(f"C[{'{c_off}'} + ", "C[off + ") for l in full_layer(T, "off", "M")]
    o.append("        r += 1")
    o.append("    }")
    o.append("    {")
    o += full_layer(T, 4 * T, "P")
    o.append("    }")
    o.append("    r = 0")
    o.append(f"    while r < {n_p} {{")
    o.append(f"        let x0 = addmod(sbox(s0), C[{5 * T} + r], R)")
    o.append(f"        let base = {2 * T - 1} * r")
    terms = ["mulmod(S[base], x0, R)"] + [f"mulmod(S[base + {j}], s{j}, R)" for j in range(1, T)]
    # lane 0 uses the OLD lanes 1.., so compute it before they are overwritten
    o.append(f"        let n0 = {dot(terms)}")
    for k in range(1, T):
        o.append(f"        s{k} = addmod(s{k}, mulmod(x0, S[base + {T + k - 1}], R), R)")
    o.append("        s0 = n0")
    o.append("        r += 1")
    o.append("    }")
    o.append("    r = 0")
    o.append("    while r < 3 {")
    o.append(f"        let off = {5 * T + n_p} + r * {T}")
    o += [l.replace(f"C[{'{c_off}'} + ", "C[off + ") for l in full_layer(T, "off", "M")]
    o.append("        r += 1")
    o.append("    }")
    # last round: sbox on all lanes, only lane 0 of the matrix product is the output
    for i in range(T):
        o.append(f"    let z{i} = sbox(s{i})")
    terms = [f"mulmod(M[{j * T}], z{j}, R)" for j in range(T)]
    o.append(f"    {dot(terms)}")
    o.append("}")
    import re
    text = "\n".join(o)
    # refer to the global const tables directly (a `let` alias would copy the array)
    text = re.sub(r"\bC\[", C + "[", text)
    text = re.sub(r"\bS\[", S + "[", text)
    text = re.sub(r"\bM\[", M + "[", text)
    text = re.sub(r"\bP\[", P + "[", text)
    return text


print("""//! GENERATED FILE, do not edit: `scripts/gen-poseidon-unrolled.py`.
//!
//! circomlib Poseidon permutation, t = 2..6, with the state in scalar locals and the round
//! bodies fully unrolled over the lanes. Same arithmetic as `poseidon::permute_array`
//! (the readable array version); see `poseidon.fe` for the safety argument.
//!
//! Bounds (safety rule 7): every `mulmod` / `addmod` result is < R. R < 0.1889 * 2^256, so a
//! plain sum of at most 5 such values is < 5 * R < 2^256 and cannot overflow; `dot` sums at most
//! 5 products before one `% R`, and for 6 terms uses `addmod` on two sums of 3 (exact for any
//! u256 operands). Round-constant and matrix entries are canonical (< R), checked by the test
//! `poseidon_tables_are_canonical`. Callers pass canonical inputs (typed `Bn254Fr`).
use std::evm::crypto::{addmod, mulmod}
use super::poseidon_constants::{
    POSEIDON_C2, POSEIDON_C3, POSEIDON_C4, POSEIDON_C5, POSEIDON_C6,
    POSEIDON_M2, POSEIDON_M3, POSEIDON_M4, POSEIDON_M5, POSEIDON_M6,
    POSEIDON_P2, POSEIDON_P3, POSEIDON_P4, POSEIDON_P5, POSEIDON_P6,
    POSEIDON_S2, POSEIDON_S3, POSEIDON_S4, POSEIDON_S5, POSEIDON_S6,
}

const R: u256 = 21888242871839275222246405745257275088548364400416034343698204186575808495617

fn sbox(_ x: u256) -> u256 {
    let x2 = mulmod(x, x, R)
    let x4 = mulmod(x2, x2, R)
    mulmod(x4, x, R)
}
""")
for T in range(2, 7):
    print(gen(T))
    print()
