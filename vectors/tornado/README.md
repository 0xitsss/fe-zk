# Tornado Cash classic withdraw proofs (real mainnet data)

Verifier-only fixtures: the Groth16 `withdraw` verifying key of Tornado Cash classic and real proofs taken from
Ethereum mainnet `withdraw` transactions of the 0.1 ETH pool. Regenerate and check everything with
`scripts/regen-tornado.sh` (`FETCH=1` re-reads the calldata from an RPC).

## Sources

| item | value |
|---|---|
| repo | `https://github.com/tornadocash/tornado-core`, commit `1ef6a263ac6a0e476d063fcb269a9df65a1bd56a` (master, 2022-03-24) |
| `contracts/Verifier.sol` | sha256 `a2fd9a12dc008ae95e1f9da2968e7d017617e804f22aa0491d2e8a91d965a824` (header: "Submitted for verification at Etherscan.io on 2020-05-12", SPDX MIT, (c) 2017 Christian Reitwiessner / 2019 OKIMS) |
| `contracts/Tornado.sol` | sha256 `64e0d454d6c077c0ae14f3e826378ad09bae57f1642e8dc1b0824a33a19c2014` (public input order, `withdraw` signature) |
| repo licence | GPL-3.0 (`LICENSE` sha256 `3972dc97...6986`). The `.sol` files are not copied into this repo; only the numeric key constants (converted) and mainnet calldata (public chain data) are committed |
| pool | `0x12D66f87A04A9E220743712cE6d9bB1B5616B8Fc`: labelled "Tornado.Cash: 0.1 ETH" by Blockscout; `denomination()` on chain returns `0x16345785d8a0000` = 0.1 ETH |
| pool verifier | `verifier()` on chain = `0xce172ce1f20ec0b3728c9965470eaf994a03557a`; all 28 key coordinates of the converted key occur in its deployed bytecode |
| solc | 0.7.6+commit.7338295f static binary, sha256 `bd69ea85427bf2f4da74cb426ad951dd78db9dfdd01d791208eccc2d4958a6bb` (binaries.soliditylang.org `list.json`), pragma of `Verifier.sol` is `^0.7.0` |

Transactions (`txs.txt`, all successful `withdraw` calls to the pool, found with the Blockscout API, calldata read with
`eth_getTransactionByHash` on `https://ethereum-rpc.publicnode.com`, no key, nothing sent):
see `txs.txt`; decoded into `proofs/<tx>.json` by `fetch.py` (`python3 -I`, stdlib only).

## Encoding (checked against Verifier.sol and by real proofs that verify)

* `withdraw(bytes _proof, bytes32 _root, bytes32 _nullifierHash, address _recipient, address _relayer, uint256 _fee, uint256 _refund)`,
  selector `0x21a0adb6`. `_proof` is 256 bytes = `abi.decode(proof, (uint256[8]))`.
* Words: `A = (p0, p1)`, `B = ([p2, p3], [p4, p5])`, `C = (p6, p7)`. Verifier.sol passes B straight to the pairing
  precompile, so B is `[[x_imag, x_real], [y_imag, y_real]]` (EIP-197 order), the same as snarkjs calldata and as the
  `verifyProof` message of generated Fe verifiers. snarkjs `proof.json` stores `[real, imag]` (`to_snarkjs.py` swaps).
* Verifier.sol also stores the key's G2 points as `[imag, real]`; `convert_vk.py` swaps them to snarkjs `[real, imag]`.
* Public inputs: `[root, nullifierHash, recipient, relayer, fee, refund]` (addresses as uint256), as in `Tornado.withdraw`.
* Verifier.sol checks every proof word `< q` and every input `< r` (it does not check G1 on-curve or G2 subgroup itself; the
  precompiles do).

## Results

* every fetched proof: `snarkjs groth16 verify` OK against `verification_key.json`;
* `vectors/tornado/crosscheck.py`: Fe verifier and Verifier.sol agree on 5 valid proofs and 231 mutations (each proof word +1, +q, = 0; each input +1, +r, = 0; B real/imag swapped; A/C swapped; inputs swapped; inputs/recipient/relayer of another tx), 0 disagreements. Execution gas of one proof: Fe 231,117 vs Verifier.sol 236,365 (revm, Osaka rules).
