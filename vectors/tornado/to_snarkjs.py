#!/usr/bin/env python3
"""proofs/<tx>.json (mainnet calldata words) -> snarkjs proof.json + public.json.

usage: python3 -I to_snarkjs.py proofs/<tx>.json OUTDIR
Calldata B = [[x_imag, x_real], [y_imag, y_real]]; snarkjs proof.json stores [real, imag], so pairs are swapped.
"""
import json
import os
import sys

d = json.load(open(sys.argv[1]))
w = d["proof_words"]
proof = {
    "pi_a": [w[0], w[1], "1"],
    "pi_b": [[w[3], w[2]], [w[5], w[4]], ["1", "0"]],
    "pi_c": [w[6], w[7], "1"],
    "protocol": "groth16",
    "curve": "bn128",
}
os.makedirs(sys.argv[2], exist_ok=True)
json.dump(proof, open(os.path.join(sys.argv[2], "proof.json"), "w"), indent=1)
json.dump(d["public"], open(os.path.join(sys.argv[2], "public.json"), "w"), indent=1)
