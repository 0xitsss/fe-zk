#!/usr/bin/env bash
# Deployed (runtime) code size guard: `fe build` every ingot given as an argument and print one line
# per contract. Exits 1 if any contract exceeds the EIP-170 limit of 24,576 bytes.
#   scripts/code-size.sh examples/halo2/circuit_verifier examples/life ...
# Env: FE (fe binary), FE_OPT (fe -O level, default 1), LIMIT (default 24576),
#      SIZE_WORK (scratch dir, default mktemp; removed on exit).
set -euo pipefail
cd "$(dirname "$0")/.."
FE="${FE:-fe}"
FE_OPT="${FE_OPT:-1}"
LIMIT="${LIMIT:-24576}"
if [ -z "${SIZE_WORK:-}" ]; then
  SIZE_WORK="$(mktemp -d)"
  trap 'rm -rf "$SIZE_WORK"' EXIT
fi
mkdir -p "$SIZE_WORK"
pids=()
i=0
for d in "$@"; do
  "$FE" build "$d" -O "$FE_OPT" --out-dir "$SIZE_WORK/$i" >/dev/null 2>"$SIZE_WORK/$i.err" &
  pids+=($!)
  i=$((i + 1))
done
fail=0
i=0
printf '%-48s %-34s %8s  %s\n' ingot contract bytes status
for d in "$@"; do
  if ! wait "${pids[$i]}"; then
    echo "fe build failed for $d" >&2
    cat "$SIZE_WORK/$i.err" >&2
    exit 1
  fi
  for f in "$SIZE_WORK/$i"/*.runtime.bin; do
    # the .bin file is hex text: two characters per byte
    n=$(( $(tr -d '[:space:]' < "$f" | wc -c) / 2 ))
    status=ok
    if [ "$n" -gt "$LIMIT" ]; then status="OVER EIP-170 ($LIMIT)"; fail=1; fi
    printf '%-48s %-34s %8d  %s\n' "$d" "$(basename "$f" .runtime.bin)" "$n" "$status"
  done
  i=$((i + 1))
done
exit "$fail"
