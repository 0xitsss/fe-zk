// Prints the creation bytecode (hex) of circomlibjs' on-chain Poseidon for <nInputs> inputs.
// Usage: NODE_MODULES_DIR=<dir with circomlibjs installed> node poseidon_code.mjs <nInputs>
import path from "node:path";
import { pathToFileURL } from "node:url";

const nm = process.env.NODE_MODULES_DIR;
const mod = await import(pathToFileURL(path.join(nm, "circomlibjs", "src", "poseidon_gencontract.js")).href);
process.stdout.write(mod.createCode(Number(process.argv[2])).replace(/^0x/, "") + "\n");
