#!/usr/bin/env python3
"""Fetch real Tornado Cash classic 0.1 ETH `withdraw` calldata from Ethereum mainnet (read-only) and decode it.

usage: python3 -I fetch.py [RPC_URL]          (default https://ethereum-rpc.publicnode.com, no key)

For every hash in txs.txt: eth_getTransactionByHash + eth_getTransactionReceipt, checks to == POOL, status == 1,
selector == withdraw(bytes,bytes32,bytes32,address,address,uint256,uint256), then writes proofs/<tx>.json:
  proof_words : the 8 words of `_proof` (A.x, A.y, B.x1, B.x0', B.y1, B.y0', C.x, C.y) in Verifier.sol order, i.e.
                B = [[x_imag, x_real], [y_imag, y_real]] (EIP-197 order = snarkjs calldata order)
  public      : [root, nullifierHash, recipient, relayer, fee, refund] as decimal strings
Nothing is ever sent to the network except JSON-RPC reads.
"""
import json
import os
import sys
import urllib.request

POOL = "0x12d66f87a04a9e220743712ce6d9bb1b5616b8fc"  # Tornado.Cash 0.1 ETH (denomination() == 1e17 checked on chain)
SELECTOR = "21a0adb6"  # withdraw(bytes,bytes32,bytes32,address,address,uint256,uint256)
HERE = os.path.dirname(os.path.abspath(__file__))
URL = sys.argv[1] if len(sys.argv) > 1 else "https://ethereum-rpc.publicnode.com"


def rpc(method, params):
    req = urllib.request.Request(
        URL,
        json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
        {"content-type": "application/json", "user-agent": "fe-zk-fetch/1"},
    )
    r = json.load(urllib.request.urlopen(req, timeout=60))
    assert "result" in r and r["result"] is not None, r
    return r["result"]


def word(data, i):
    return int.from_bytes(data[32 * i : 32 * i + 32], "big")


def decode(inp):
    assert inp.startswith("0x" + SELECTOR), "not a withdraw call"
    d = bytes.fromhex(inp[10:])
    assert len(d) >= 7 * 32
    off = word(d, 0)
    assert off == 7 * 32, "unexpected head layout"
    n = word(d, off // 32)
    assert n == 256, "proof must be 8 words"
    proof = [word(d, off // 32 + 1 + i) for i in range(8)]
    root, nf, rec, rel, fee, refund = (word(d, i) for i in range(1, 7))
    assert rec >> 160 == 0 and rel >> 160 == 0, "address word has dirty high bits"
    return proof, [root, nf, rec, rel, fee, refund]


os.makedirs(os.path.join(HERE, "proofs"), exist_ok=True)
for line in open(os.path.join(HERE, "txs.txt")):
    h = line.split("#")[0].strip()
    if not h:
        continue
    tx = rpc("eth_getTransactionByHash", [h])
    rc = rpc("eth_getTransactionReceipt", [h])
    assert tx["to"].lower() == POOL, "wrong pool"
    assert int(rc["status"], 16) == 1, "tx reverted"
    proof, pub = decode(tx["input"])
    out = {
        "tx": h,
        "block": int(tx["blockNumber"], 16),
        "pool": POOL,
        "proof_words": [str(x) for x in proof],
        "public": [str(x) for x in pub],
    }
    with open(os.path.join(HERE, "proofs", h + ".json"), "w") as f:
        json.dump(out, f, indent=1)
        f.write("\n")
    print(h, "block", out["block"], "ok")
