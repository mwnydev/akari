# AI audit pack

> 用途：ADR 0007 第 2 条「多方 AI 审计」的提示词包。内容用英文写，直接整段粘贴给各个模型；每个模型独立跑，结果按第 7 节分诊。

How to use: run `contracts/script/audit-bundle.sh` to produce `contracts/out/audit-bundle.txt` (this file, the freeze
checklist, threat model, ADRs, every in-scope source, the library interfaces they use and the test list). Give each
model **sections 1–6 of this file as the instructions** and the bundle as the material. One model, one fresh session,
no access to other models' findings.

## 1. Role and rules

You are an independent smart-contract security auditor acting as an attacker. You have the full source of a
non-upgradeable escrow system that will hold real USDT on Arbitrum One. There is no paid audit; your findings, the
symbolic checks and a public review are the only line of defence, and the on-chain caps (200 USDT per order, 5000 USDT
held in total) bound the worst case.

- Report only issues you can turn into a **failing Foundry test** against the given code (section 6). If you cannot
  write the test, say so and mark the finding "needs-confirmation".
- Assume every external caller is hostile, including a buyer and seller who are the same person, a malicious smart
  account as seller (ERC-1271 / ERC-6492), and a leaked forum-sponsor key. Do **not** assume the owner, arbiter or
  operations Safe (2-of-3) is compromised; do flag anything those roles can do beyond the powers table in
  freeze-checklist.md §5.
- Known limitations and accepted risks are listed in freeze-checklist.md §8. Do not report them again unless you can
  show a consequence worse than what is written there.
- Prefer depth over breadth: trace every token transfer and every `totalLocked` update end to end.
- No gas optimisations, style comments or "consider using X" without an exploit.

## 2. System summary

Escrow trading for a forum. Buyers pay USDT (USD₮0, 6 decimals) into an escrow contract; an order can only end with
money going to its buyer, its seller and the platform fee recipient (fees, base gas, dispute fees).

| Contract | Role |
| --- | --- |
| `EscrowConfig` | Owner = treasury Safe. Fee / cap / timeout parameters within hard-coded bounds; tightening waits 72 h (queued, cancellable, applied by anyone); pause (new orders only); fee campaigns with gas-subsidy slots; token whitelist (72 h); auction signer (72 h); market list (72 h, no removal). Tracks `totalLocked` across markets and enforces `totalLockedCap` on new orders and bond deposits. |
| `Escrow` | One-time escrow (`createAndFund` → `markDelivered` → `release` / `claimTimeout`; `refund`; `proposeEnd` / `acceptEnd`; `dispute` → `resolve` / `resolveTimeout`); the seller bond pool shared with StreamEscrow (`lockBond` / `releaseBond` / `takeBond` for markets); auction settlement `createFromAuction` (seller EIP-712 authorization + platform settlement proof; CNY prices converted at the seller-signed rate, rounded up). |
| `StreamEscrow` | Installment escrow: net principal unlocks per whole period after `start`; the seller `claim`s (≥ 24 h apart, minus claim gas); unlocking freezes at `disputedAt`; same endings as Escrow plus `finalize`. Arbiter, fee recipient and config are read from Escrow. |
| `MarketBase` | Offer fill counting, cancelled nonces, ERC-6492 signature checks; pull-payment fallback (`_push` credits the recipient when the token refuses, `withdraw(token, to)`); `_pullExact` (balance-delta accounting). |
| `FeeMath` | Fee, split, dispute fee, stream unlock, claim net, pro-rata fee with dispute-fee offset, CNY→USDT (`fullMulDivUp`). Must match `test-vectors/fees.json`. |
| `FixedFeePaymaster` | EntryPoint v0.7 paymaster. Validates a forum-sponsor signature over the userOp + `validUntil` / `validAfter` / `fee`; pulls `fee` (≤ 1 USDT hard cap) from the account to itself **during validation**; no `postOp`. `sweepFees` (anyone) forwards to the immutable treasury. Owner (operations Safe) rotates the signer and manages the ETH deposit / stake. |

Settlement model (both markets): every ending is a seller share `s` of the principal (Escrow: `amount`; Stream: `net`,
claimed amounts included). The fee pool (fees + base gas, kept apart from principal at creation) is earned pro rata to
`s`; base gas is earned in full once the platform sponsored any post-payment operation (`platformPaid`); a retained
dispute fee replaces the platform fee (at most one of the two is charged). The buyer gets the remainder.

## 3. Threat model excerpt (docs/threat-model.md)

| Id | Threat | Attacker | Intended defence |
| --- | --- | --- | --- |
| T-04 | An admin path that moves escrowed funds | insider (E5) | No withdrawal function, not upgradeable, no selfdestruct; conservation invariant |
| T-05 | Offer signature replay (cross-chain, cross-contract, cross-buyer, beyond fill count) | user (E1) | EIP-712 domain with chainId + contract; designated buyer; fill count per offer hash; nonce + `cancelOffer` |
| T-06 | Old offer charged at raised fees | insider (E5) | Offer carries max fee bps / min fee / base gas (/ claim gas); parameters snapshotted per order; raises wait 72 h |
| T-07 | Amount passed ≠ amount received (fee-on-transfer tokens) | user (E1) | Balance-delta accounting (`_pullExact`) |
| T-08 | USDT freezes one address and blocks other orders | chain (E7) | Pull-payment fallback + `withdraw(token, to)` |
| T-09 | Stream fee underflow late in the stream | user (E1) | Fee pool set aside at creation |
| T-10 | Failing ops drain the paymaster deposit | user (E1) | Fee pulled during validation (callData cannot undo it); signature window ≤ 10 min |
| T-15 | Arbiter colludes with one party | insider (E5) | 2-of-3 Safe; `resolve` can only split between buyer and seller |
| T-19 | Sequencer outage makes deadlines expire | chain (E7) | Windows ≥ 24 h; arbiter `extendDeadlines` (extend only, ≤ 7 d per call, ≤ 14 d total) |
| T-20 | Auction settlement key stolen | server (E4) | Can only sell at ≥ reserve to someone else, who pays; each auction settles once |

Attackers: E1 any user (any account, any public function, smart-account sellers); E4 full server compromise (all
server keys: the paymaster sponsor signer and the auction signer); E5 a malicious insider holding at most one Safe key;
E7 chain-level failures (sequencer down, USDT freezing an address).

## 4. Invariants to break

Ids match freeze-checklist.md §6, where each is mapped to its tests and symbolic checks.

1. **I1 Conservation**: per order, everything paid in (principal + buyer fee + buyer gas + dispute fee) = paid to buyer + seller + fee recipient + still held. Per contract, token balance = open-order holdings + seller bonds + unclaimed credits.
2. **I2 Recipients**: an order's funds only ever reach its buyer, its seller or the immutable fee recipient (directly, or via a credit withdrawn by that same account).
3. **I3 Caps**: no order above `orderCap` (200 USDT at launch) is created; `totalLocked` (open holdings + bonds) never exceeds `totalLockedCap` (5000 USDT) through `createAndFund`, `createFromAuction` or `depositBond`; `totalLocked` always equals open holdings + bonds.
4. **I4 Signed fee caps**: fee ≤ max(amount × `maxFeeBps`, `maxMinFee`), base gas ≤ `maxBaseGas`, claim gas ≤ `maxClaimGas`, whatever the owner configured and whatever campaign runs.
5. **I5 One charge**: per order the fee recipient gets base gas (only if `platformPaid`) plus at most one of {platform fee, dispute fee}; claim gas only per successful claim.
6. **I6 Liveness**: every open order has an exit nobody can block: `claimTimeout` / `finalize` / `resolveTimeout` are callable by anyone; the arbiter can always `resolve` a disputed order; settlements never revert for in-range inputs (no underflow, no stuck dust).
7. **I7 Streams**: `claimed ≤ unlocked ≤ net`; `unlocked` is monotonic in time, frozen at `disputedAt`, and reaches `net` at the end.
8. **I8 Bonds**: a seller's locked bond = sum of the bonds of their open orders (across both markets); only the free part can be withdrawn or used for a dispute fee; a slashed bond only goes to that order's buyer.
9. **I9 Auctions**: an auction (seller, auctionId) settles at most once; payer = proof winner; price ≥ reserve in the auction currency; CNY amount = ceil(price × 10^8 / rate) with the seller-signed rate in [5, 10].
10. **I10 Admin limits**: owner changes never touch existing orders; raising charges or tightening rules waits 72 h; hard caps are constants; pause blocks only new orders; there is no path for the owner, the arbiter or a market to take principal.
11. **I11 Offers**: an offer fills at most `maxFills` times, only before `expiry`, only by the designated buyer (if set), never after `cancelOffer`, never on another chain or contract.
12. **I12 Paymaster**: a fee moves only when the sponsor signature is valid; it equals the signed fee and is ≤ 1 USDT; `validUntil ≠ 0` and the window is ≤ 10 minutes; collected fees can only go to the treasury; only the EntryPoint can trigger validation.

## 5. Per-contract focus questions

Answer each question explicitly ("checked, holds because …" or a finding).

**FeeMath / settlement arithmetic**
- Can `_shares` (Escrow or Stream) underflow for any reachable (s, fees, gas, `disputeRetained`, `platformPaid`) combination, e.g. buyer-pays fees with a small `s`, or `Half` splits with odd totals? A revert here locks the order until another ending is possible.
- `feesCharged` scales the floored buyer part; can rounding ever make `feeB + feeS` exceed what the order holds, or move more than 1 unit between buyer and seller?
- `cnyFenToUsdt`: any price / rate where the seller gets less than `price × 10^8 / rate`? Any overflow or revert for prices that pass `orderCap`?
- Stream: `streamUnlocked` with `periods` up to 8760 and `net` near `orderCap`; remainder handling in the last period; the `claimNet` boundary at exactly `claimGas`.

**Escrow**
- State machine: can any function act on a `Released` / `Refunded` / `Settled` order, or settle an order twice (including `claimTimeout` vs `acceptEnd` races, or reentrancy through the token or an ERC-1271 seller)?
- `extendConfirm` / `extendDeadlines`: can a seller or the arbiter push a deadline beyond the documented caps, or make `claimTimeout` impossible?
- Disputes: wallet- vs bond-paid dispute fees; the refund path back to the bond (`disputeFromBond`) vs the wallet; `resolve` bps edge cases (0, 10 000) for buyer- and seller-initiated disputes; `resolveTimeout` for delivered vs undelivered orders.
- Bonds: `lockBond` / `releaseBond` / `takeBond` from a market; can one market release another market's lock, or a seller withdraw bond still locked by a stream order?
- Auctions: can two different seller authorizations for the same `auctionId` both settle? Can a proof be reused across sellers, tokens or currencies? What can a stolen auction-signer key do beyond T-20?
- `totalLocked`: is every token inflow and outflow mirrored (orders, bonds, dispute fees, slashes, credits)? Can `unlockFunds` underflow and brick settlements?

**StreamEscrow**
- Can the seller claim more than unlocked, claim after a dispute, or claim so as to make `acceptEnd` / `resolve` revert?
- `FeeSource.Unlocked`: the dispute fee is taken out of unlocked principal by bumping `claimed`; check every ending afterwards (refund, resolve both ways, resolveTimeout, acceptEnd) for double counting.
- `finalize` vs `claim` after the end; `refund` after partial claims; `claimTimeout` vs `start` at the deadline boundary.
- Bond release ordering in `_settle` (an external call into Escrow before the pushes) and reentrancy.

**EscrowConfig**
- `_isLoosening`: is there a field combination that raises a charge or tightens a rule yet applies immediately?
- Campaigns: `_atLeastAsGenerous`, subsidy slot counting, ending a campaign early, scopes; can the owner change an existing order's charges retroactively?
- Queue keys: can a stale queued token / signer / market / campaign change be applied after the state it was queued against has changed?
- Can a non-market call `checkout`, `lockFunds`, `unlockFunds`? Can the owner add a market without the delay?

**MarketBase**
- `_useOffer` increments `fills` before the signature check; is there a path where a failed check does not revert the increment?
- `_push` / `withdraw`: can a credit be redirected, withdrawn twice, or created for an address that is not a party?
- `_pullExact` with fee-on-transfer, rebasing or callback tokens — whitelisted tokens are 6-decimal ERC-20s, but state what breaks if a hostile token were whitelisted.

**FixedFeePaymaster**
- Can a userOp be accepted with a fee different from the signed one, above 1 USDT, with `validUntil == 0`, or with a window longer than 10 minutes?
- Does `getHash` cover every field an attacker could change to reuse a signature (sender, nonce, initCode, callData, gas fields, paymaster gas limits, chain, paymaster address)? Note that `paymasterAndData[20:52]` is hashed while `[52:80]` is hashed as separate values.
- Can the fee be pulled when account validation later fails (AA24) or when the op is never executed? Can callData front-run the pull?
- What can the owner (operations Safe) do with user funds? What can a stolen sponsor key do?
- ERC-7562: what breaks under a public bundler (proxy token implementation-slot reads)?

## 6. Required output format

Return Markdown with one block per finding, most severe first, then the answers to section 5.

```
### <MODEL>-<NN>: <one-line title>
- Severity: Critical | High | Medium | Low | Info | needs-confirmation
- Invariant: I<n> (section 4), or "none"
- Location: contracts/src/<File>.sol:<line>[-<line>] (<function>)
- Preconditions: who calls what, in which state, with which parameters
- Exploit sketch: numbered steps, with the amounts that move and to whom
- Impact: funds lost / locked (whose, how much, bounded by which cap), or what else breaks
- Failing test: a complete Foundry test function that fails on the current code because it asserts the correct
  behaviour (and passes once fixed). Reuse the fixtures in contracts/test/EscrowBase.sol, StreamBase.sol and
  aa/AATestBase.sol.
- Fix: minimal code change (diff) and why it restores the invariant
```

Severity guide (same as docs/audit/bug-bounty.md): Critical = others' funds stolen or permanently locked across orders,
or caps bypassed; High = one order's funds stolen or permanently locked, an auction settled twice or below reserve,
fees above the signed caps; Medium = funds delayed beyond documented deadlines, repeatable rounding loss > 1 unit,
paymaster deposit drained without a key; Low = deviation from the spec without loss.

## 7. Procedure and triage

**Run** (one round = every model below, independently):

1. Run `contracts/script/audit-bundle.sh` on the frozen commit; record the commit and the bundle's sha256.
2. For each model (at least three, from different vendors / model families), open a fresh session, paste sections 1–6
   as the instructions and the bundle as the material. If the model's context is small, run one pass per contract in
   the order FeeMath → MarketBase → EscrowConfig → Escrow → StreamEscrow → FixedFeePaymaster, always with sections 1–6.
3. Follow up once per model: "List three more attacks you considered and rejected, and why each fails." This surfaces
   reasoning that can be checked.
4. Save each raw answer unedited as `docs/audit/round-<n>/<model>.md`.

**Triage** (one owner per round):

1. **Dedupe**: group findings by root cause (same location + same broken invariant); keep the clearest write-up and
   list the duplicates' ids under it.
2. **Verify**: put the finding's test into `contracts/test/audit/Round<n>.t.sol` and run
   `forge test --match-path 'test/audit/*'`. A finding is confirmed only when its test fails on the frozen code.
   Rejected findings get a one-line reason (e.g. "precondition unreachable: `_checkTerms` rejects bond > amount").
3. **Fix**: the smallest change that restores the invariant; the confirming test stays in the suite. Every fix follows
   the post-freeze change policy (freeze-checklist.md §11): all gates, new file hashes, a mutation run for the touched
   contract, the symbolic checks for the touched properties.
4. **Re-run**: after the fixes, run a new round with every model on the new bundle, adding the previous round's
   confirmed findings as regression questions ("Is <finding> fixed? Can the fix be bypassed?").
5. **Record** every finding in `docs/audit/findings.md`:

| Id | Round | Models | Severity | Status (confirmed / rejected / duplicate / fixed) | Test | Fix commit |
| --- | --- | --- | --- | --- | --- | --- |

Stop when a full round produces no new confirmed finding of Medium or above.
