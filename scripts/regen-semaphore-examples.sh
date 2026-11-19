#!/usr/bin/env bash
# Regenerate the Semaphore v4 proofs and the tests that use them:
#   1. fetch the real ceremony artifacts semaphore-<d>.{zkey,wasm}, d = 1..10, from snark-artifacts.pse.dev
#      (@zk-kit/artifacts Project.SEMAPHORE 4.13.0; sha256 pinned below; never committed)
#   2. prove with the official SDK (@semaphore-protocol/proof 4.14.5, lockfile in vectors/js/semaphore):
#      one proof per depth + the vote / claim / signal example proofs  ->  vectors/semaphore/
#      (proofs are randomized: rerunning changes the committed JSON, the tests stay valid)
#   3. regenerate semaphore/src/tests.fe and examples/anon_*/src/tests.fe from them, format
#   4. cross-check: all proofs are accepted by Semaphore's unmodified SemaphoreVerifier.sol in revm
#      (scripts/check-semaphore-sol.sh)
# Env: CACHE (default vectors/cache/semaphore), FE.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CACHE="${CACHE:-$ROOT/vectors/cache/semaphore}"
FE="${FE:-fe}"
ART=https://snark-artifacts.pse.dev/semaphore/4.13.0
mkdir -p "$CACHE"
fetch() { # file sha256
  [ -f "$CACHE/$1" ] || curl -sSfL -o "$CACHE/$1" "$ART/$1"
  echo "$2  $CACHE/$1" | sha256sum -c - >/dev/null || { echo "sha256 mismatch: $1" >&2; exit 1; }
}
fetch semaphore-1.wasm 70289ef3bdfa86e97debdb227e76721b392f45b5e73d5e7136d93a04dce0dff9
fetch semaphore-1.zkey 471a978875f91411b21ba5a2fbea41df30ec45eb23e3ac045edf7f30a2846a08
fetch semaphore-2.wasm 877a989a8881ff6651866cfa32088f60e4abcb02a26f2e995868dfabd7f9385b
fetch semaphore-2.zkey 110e2d9de78730aa4f5c78c75017cfe4d769b4b7ce60f58484fb14733d7b0f35
fetch semaphore-3.wasm 48e15502f710be0a623d573d472edeeaf918fd5eee0b2ca9b407c4e4f20d12f2
fetch semaphore-3.zkey c36653c42784df35a01f3d93415af9ad8292a540f8deb134a6a34a01752a89d3
fetch semaphore-4.wasm ec0ed0ae7eb0264070b36a247cf4aad8157c5550f0731d3462019526e5e9ae4f
fetch semaphore-4.zkey 90c94bad7ad97eeb9314750e443d84da783e38a21f55538d72b10d258064f6e6
fetch semaphore-5.wasm 74ee373fc66bf81ca89de52602b555f16ef0e54c0e36e5b69c6e109d146acfb5
fetch semaphore-5.zkey a1431c24dc49e98d07d48e7bdacc002875bcb02a799d9156c6aa6bcbe0838926
fetch semaphore-6.wasm 432293aa5c8e7d503148824ff17a8ad7318058a656af568837f30675d867827e
fetch semaphore-6.zkey 24a7ef2c7f4c809a502e876cb3295535a73710ec7bc3827b5a15018e434a674a
fetch semaphore-7.wasm 3ba75768d5dfbee9d52b732b69eefb1725c8c356a3de9c2f2a71dbc401a0ea61
fetch semaphore-7.zkey d77851ee4bd496c6b5f70fbfa550f113a89bad972efd4c541267e566163d0600
fetch semaphore-8.wasm 4edc07444a1d48d09014c963e315d3351b6e12838d83fcb1962071614b1c94b9
fetch semaphore-8.zkey d02521afe7133a97421244374d4bc005c1ebd7d7cf7c51e88b7e99fdaf183146
fetch semaphore-9.wasm a75f9d92dc826629e3490596ec99055bbecc006050dee6bf8e81b99c9a54f16a
fetch semaphore-9.zkey 0eae4df76414b885a2f0793740330de896ab13f07504f97c81b5fe92868ce90e
fetch semaphore-10.wasm 1e9e1e3ad9917164bc96c751e32fae724400b92063f5a2cfd22ca64674bf4f96
fetch semaphore-10.zkey f644af6753be9d48454d8f9bb74bf2e9a416dd7d1a704c3cf1d50e535969a19a

(cd "$ROOT/vectors/js/semaphore" && { [ -e node_modules ] || npm ci --no-audit --no-fund >/dev/null; } &&
  node prove-examples.mjs "$CACHE" "$ROOT/vectors/semaphore")
python3 -I "$ROOT/scripts/gen-semaphore-tests.py"
python3 -I "$ROOT/scripts/gen-anon-example-tests.py"
(cd "$ROOT/semaphore" && "$FE" fmt >/dev/null)
for e in vote claim signal; do (cd "$ROOT/examples/anon_$e" && "$FE" fmt >/dev/null); done
CACHE="$CACHE" "$ROOT/scripts/check-semaphore-sol.sh"
echo "semaphore proofs and tests regenerated"
