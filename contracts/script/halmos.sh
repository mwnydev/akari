#!/usr/bin/env bash
# Runs the Halmos specs in test/symbolic (results and bounds: docs/audit/freeze-checklist.md §6). Solver settings live
# in each spec's `@custom:halmos` tag. With no arguments every *Symbolic contract runs; otherwise the arguments go to
# halmos as is, e.g. `script/halmos.sh --contract EscrowSymbolic --match-test '^check_release\('`.
# The solvers are memory-hungry: on a shared machine cap the run, e.g.
#   systemd-run --user --scope -p MemoryMax=4G script/halmos.sh
set -euo pipefail
cd "$(dirname "$0")/.."

# Halmos needs the AST in the artifacts; a separate artifact directory keeps it from invalidating the normal build.
export FOUNDRY_OUT=cache/halmos/out FOUNDRY_CACHE_PATH=cache/halmos/cache
export HALMOS_ALLOW_DOWNLOAD=1 # the cvc5 build halmos pins is fetched on first use

args=("$@")
[ ${#args[@]} -gt 0 ] || args=(--match-contract 'Symbolic$')
exec uvx --from halmos==0.3.3 halmos --forge-build-out cache/halmos/out --solver-threads "${HALMOS_THREADS:-4}" \
  "${args[@]}"
