#!/usr/bin/env bash
# Builds one plain-text bundle of the frozen contracts for pasting into an AI auditor (docs/audit/ai-audit-pack.md):
# the audit prompt pack, in-scope sources, the interfaces they import from lib/, the tests (names only), and the
# design docs. Everything else in lib/ is listed by path and version only.
#
# Usage: contracts/script/audit-bundle.sh [OUT]   (default: contracts/out/audit-bundle.txt; needs forge and jq)
set -euo pipefail

contracts=$(cd "$(dirname "$0")/.." && pwd)
repo=$(dirname "$contracts")
out=${1:-$contracts/out/audit-bundle.txt}

# Freeze scope (docs/audit/freeze-checklist.md §1). MarketBase, EscrowConfig and FeeMath come in as dependencies.
scope=(Escrow StreamEscrow FixedFeePaymaster)
docs=(
  docs/audit/ai-audit-pack.md
  docs/audit/freeze-checklist.md
  docs/threat-model.md
  docs/adr/0004-paymaster-minimal-fixed-fee.md
  docs/adr/0007-no-paid-audit.md
  docs/adr/0011-cny-pricing.md
  contracts/src/README.md
  contracts/src/aa/SECURITY.md
)
# The paymaster's access control (`onlyEntryPoint`, deposit / stake management) lives in its parent contract.
extra_lib=(lib/account-abstraction/contracts/core/BasePaymaster.sol)

cd "$contracts"
forge build --skip test --skip script >/dev/null 2>&1 || { echo "forge build failed" >&2; exit 1; }

# Every source file compiled into the in-scope contracts, from the compiler metadata.
sources=$(for c in "${scope[@]}"; do
  jq -r '.metadata.sources | keys[]' "out/$c.sol/$c.json"
done | sort -u)
src_files=$(grep '^src/' <<<"$sources")
lib_files=$(grep '^lib/' <<<"$sources")
# Interfaces: files under an interfaces/ directory or named I<Upper>*.sol.
iface_files=$(grep -E '/interfaces/|/I[A-Z][A-Za-z0-9]*\.sol$' <<<"$lib_files" || true)

section() { printf '\n\n================================================================\n= %s\n================================================================\n' "$1"; }
file() { printf '\n----- %s -----\n' "$1"; cat "$2"; }

mkdir -p "$(dirname "$out")"
{
  echo "Akari escrow contracts — audit bundle"
  echo "Generated from $(git -C "$repo" describe --always --dirty 2>/dev/null || echo unknown) on $(date -u +%Y-%m-%dT%H:%MZ)"
  echo "Compiler settings (foundry.toml): $(grep -E '^(solc_version|optimizer|optimizer_runs|via_ir|evm_version) ' foundry.toml | tr '\n' ' ')"
  echo "Libraries: OpenZeppelin $(jq -r .version lib/openzeppelin-contracts/package.json), Solady $(jq -r .version lib/solady/package.json), eth-infinitism account-abstraction v0.7.0 (vendored)"

  section "DOCUMENTS"
  for d in "${docs[@]}"; do file "$d" "$repo/$d"; done

  section "IN-SCOPE SOURCES (contracts/src)"
  for f in $src_files; do file "contracts/$f" "$f"; done
  file "contracts/src/MockUSDT.sol (testnet only, NOT deployed on mainnet)" src/MockUSDT.sol

  section "LIBRARY INTERFACES AND PARENTS USED BY THE SOURCES"
  for f in $iface_files "${extra_lib[@]}"; do file "contracts/$f" "$f"; done

  section "OTHER LIBRARY FILES COMPILED IN (not included; audit status in docs/audit/freeze-checklist.md §3)"
  grep -v -x -F -f <(printf '%s\n' $iface_files "${extra_lib[@]}") <<<"$lib_files" || true

  section "TESTS (contracts/test: test*, invariant*, check* functions)"
  find test -name '*.sol' | sort | while read -r t; do
    names=$(grep -oE 'function (test|testFuzz|invariant|check)_[A-Za-z0-9_]+' "$t" | sed 's/function //' || true)
    [ -n "$names" ] || continue
    printf '\n%s\n' "contracts/$t"
    sed 's/^/  /' <<<"$names"
  done
} >"$out"

echo "wrote $out ($(wc -c <"$out") bytes)"
