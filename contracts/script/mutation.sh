#!/usr/bin/env bash
# Mutation testing with Foundry's built-in `forge test --mutate` (results: docs/audit/freeze-checklist.md §7).
#
#   script/mutation.sh src/Escrow.sol [more src files...]
#
# Runs in a scratch copy of contracts/ (MUTATION_WORKDIR, default a new temp dir) so the real tree and its caches stay
# untouched. Per-file results are cached in <workdir>/contracts/cache/mutation; reuse the same MUTATION_WORKDIR to
# resume. MUTATION_JOBS sets the parallel workers (default 2; each needs ~1 GB).
#
# This is the screening pass: invariant suites are skipped (a mutant that breaks an invariant triggers sequence
# shrinking, which makes each such mutant take minutes) and fuzz tests run 64 times. Re-run every survivor against the
# full suite with the normal config before counting it (the checklist lists the confirmed ones).
set -euo pipefail

contracts=$(cd "$(dirname "$0")/.." && pwd)
repo=$(dirname "$contracts")
work=${MUTATION_WORKDIR:-$(mktemp -d)}
[ $# -gt 0 ] || { echo "usage: $0 src/<File>.sol..." >&2; exit 2; }

mkdir -p "$work/contracts" "$work/tmp"
rm -rf "$work/contracts/src" "$work/contracts/test" "$work/contracts/script"
cp -r "$contracts/src" "$contracts/test" "$contracts/script" "$work/contracts/"
rm -rf "$work/contracts/test/symbolic" # Halmos specs; forge test never runs them
ln -sfn "$contracts/lib" "$work/contracts/lib"
# Tests read ../test-vectors. Forge copies the project into $TMPDIR/forge_mutation_* for each worker, so the vectors
# must sit next to those copies too; otherwise every mutant is "killed" by the vector tests failing to load.
ln -sfn "$repo/test-vectors" "$work/test-vectors"
ln -sfn "$repo/test-vectors" "$work/tmp/test-vectors"
# Screening config. The gas limit stops mutants that turn a loop into an endless one before their logs fill memory.
sed -e 's/^runs = 1000$/runs = 64/' -e 's/^evm_version = "cancun"$/&\ngas_limit = 50000000/' \
  "$contracts/foundry.toml" >"$work/contracts/foundry.toml"

cd "$work/contracts"
echo "workdir: $work"
TMPDIR="$work/tmp" forge test --mutate "$@" --mutation-jobs "${MUTATION_JOBS:-2}" --mutation-timeout 300 \
  --fail-fast -j 4 --no-match-contract Invariant
