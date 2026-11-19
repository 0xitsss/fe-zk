#!/usr/bin/env python3
"""Convert the verifying key hard-coded in tornado-core's Verifier.sol to snarkjs verification_key.json.

usage: python3 -I convert_vk.py Verifier.sol verification_key.json

Verifier.sol (pinned in README.md) stores G2 coordinates in the EIP-197 precompile order [imag, real]
("Encoding of field elements is: X[0] * z + X[1]"); snarkjs stores [real, imag], so each pair is swapped.
alpha_beta (vk_alphabeta_12) is not in the Solidity file and not needed by fe-zk-gen, so it is omitted.
"""
import json
import re
import sys

src = open(sys.argv[1]).read()
U = r"uint256\((\d+)\)"


def g1(name):
    m = re.search(re.escape(name) + r" = Pairing\.G1Point\(" + U + ", " + U + r"\);", src)
    return [m.group(1), m.group(2), "1"]


def g2(name):
    m = re.search(
        re.escape(name) + r" = Pairing\.G2Point\(\[" + U + ", " + U + r"\], \[" + U + ", " + U + r"\]\);", src
    )
    xi, xr, yi, yr = m.groups()
    return [[xr, xi], [yr, yi], ["1", "0"]]


ic = [g1("vk.IC[%d]" % i) for i in range(7)]
assert "vk.IC[7]" not in src
vk = {
    "protocol": "groth16",
    "curve": "bn128",
    "nPublic": 6,
    "vk_alpha_1": g1("vk.alfa1"),
    "vk_beta_2": g2("vk.beta2"),
    "vk_gamma_2": g2("vk.gamma2"),
    "vk_delta_2": g2("vk.delta2"),
    "IC": ic,
}
json.dump(vk, open(sys.argv[2], "w"), indent=1)
open(sys.argv[2], "a").write("\n")
