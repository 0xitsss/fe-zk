#!/usr/bin/env python3
"""ABI calldata for gnark's PlonkVerifier.Verify(bytes proof, uint256[] public_inputs).
Usage: calldata.py <fixture.json> <4-byte selector hex> [tamper-word-index]  -> prints hex"""
import json, sys

d = json.load(open(sys.argv[1]))
proof = b"".join(bytes.fromhex(w[2:]) for w in d["proof_words"])
pubs = [int(p, 16) for p in d["public"]]
w = lambda x: x.to_bytes(32, "big")
pad = (-len(proof)) % 32
head = w(0x40) + w(0x40 + 32 + len(proof) + pad)
body = w(len(proof)) + proof + b"\0" * pad + w(len(pubs)) + b"".join(w(p) for p in pubs)
print(sys.argv[2] + (head + body).hex())
