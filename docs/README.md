# fe-zk documentation

| Document | Read it when |
|---|---|
| [USAGE.md](USAGE.md) | you want to verify proofs in your Fe project: install, one recipe per proof system, your own STARK AIR, deploy, troubleshooting |
| [API.md](API.md) | you need the public functions and types (generated from the source by `scripts/gen-api-docs.py`) |
| [SAFETY.md](SAFETY.md) | you integrate a verifier or review one: each safety rule, the file:line that enforces it and the test that fails without it |
| [DESIGN.md](DESIGN.md) | you want to know how it works: Fe capabilities, field and curve layer, each verifier, the Plonky3 0.8 port |
| [GAS.md](GAS.md) | you compare costs: Fe vs the reference Solidity verifiers on the same proofs, with the commands that measured them |
| [TESTING.md](TESTING.md) | you change the code: test layout, fixtures, cross-checks, CI jobs |
| [AI_LOG.md](AI_LOG.md) | you want to know what AI assistance produced and how it was checked |

Examples: [examples/README.md](../examples/README.md). Security policy: [SECURITY.md](../SECURITY.md).
