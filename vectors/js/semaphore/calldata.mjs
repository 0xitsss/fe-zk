// Prints gas-run calldata (`sig:` line + argument words) for Semaphore's published Solidity verifier
//   verifyProof(uint256[2] pA, uint256[2][2] pB, uint256[2] pC, uint256[4] pubSignals, uint256 merkleTreeDepth)
//   node calldata.mjs <depth> <proof.json> <public.json> [tamper]
import * as snarkjs from "snarkjs"
import fs from "node:fs"

const [depth, proofPath, publicPath, tamper] = process.argv.slice(2)
const proof = JSON.parse(fs.readFileSync(proofPath, "utf8"))
const pub = JSON.parse(fs.readFileSync(publicPath, "utf8"))
if (tamper) pub[1] = (BigInt(pub[1]) + 1n).toString()
// exportSolidityCallData -> "[a0,a1],[[b00,b01],[b10,b11]],[c0,c1],[p0..p3]" (pB already in the Solidity order)
const txt = await snarkjs.groth16.exportSolidityCallData(proof, pub)
const nums = JSON.parse("[" + txt + "]").flat(Infinity)
nums.push(BigInt(depth).toString())
const hex = nums.map((n) => BigInt(n).toString(16).padStart(64, "0")).join("")
console.log("sig:verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[4],uint256)")
console.log(hex)
process.exit(0)
