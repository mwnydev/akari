"use client";

import {
  Amount,
  Button,
  Card,
  CardTitle,
  Field,
  Input,
  NetworkBadge,
  formatUsdt,
  parseUsdt,
} from "@akari/ui";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { CircleCheck, KeyRound, LoaderCircle, TriangleAlert } from "lucide-react";
import { useState, type ReactNode } from "react";
import { formatEther } from "viem";
import { bundlerPresets, endpointError, rpcPresets, topUp } from "../../lib/emergency/rules";
import { loadEmergencySdk } from "../../lib/emergency/load";
import type {
  Balances,
  Connection,
  EmergencyOp,
  EmergencyWallet,
  Market,
  WalletCandidate,
} from "../../lib/emergency/sdk";
import { NETWORK_LABEL, chain } from "../../lib/wallet/config";
import { checkAddress, withdrawError } from "../../lib/wallet/withdraw";
import { TxLink } from "../TxLink";

// 紧急取回页面（ADR 0007）：不连论坛服务器。只用用户选的公共 RPC + 第三方 Bundler，gas 用钱包自己的 ETH。

const message = (e: unknown, fallback: string) =>
  e instanceof Error && /[一-鿿]/.test(e.message) ? e.message : fallback;

function ErrorLine({ error }: { error: string }) {
  return error ? (
    <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
      {error}
    </p>
  ) : null;
}

/** 预填列表 + 可自由修改的节点地址。 */
function EndpointField({
  label,
  presets,
  value,
  onChange,
}: {
  label: string;
  presets: { label: string; url: string }[];
  value: string;
  onChange: (v: string) => void;
}) {
  const error = endpointError(value);
  return (
    <Field label={label} error={value ? (error ?? undefined) : undefined}>
      <Input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        spellCheck={false}
        className="amount font-mono text-sm"
      />
      {presets.length > 1 ? (
        <span className="mt-1 flex flex-wrap gap-1">
          {presets.map((p) => (
            <Button key={p.url} size="sm" variant="secondary" onClick={() => onChange(p.url)}>
              {p.label}
            </Button>
          ))}
        </span>
      ) : null}
    </Field>
  );
}

/** 第 1 步：选节点并连接（核对链 ID 与 EntryPoint）。 */
function ConnectStep({ onConnected }: { onConnected: (c: Connection) => void }) {
  const [rpc, setRpc] = useState(rpcPresets()[0]?.url ?? "");
  const [bundler, setBundler] = useState(bundlerPresets()[0]?.url ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const invalid = endpointError(rpc) !== null || endpointError(bundler) !== null;
  async function connect() {
    setBusy(true);
    setError("");
    try {
      onConnected(await (await loadEmergencySdk()).connect(rpc.trim(), bundler.trim()));
    } catch (e) {
      setError(message(e, "连接失败，请换一个节点"));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Card className="space-y-3">
      <CardTitle>1. 选择公共节点</CardTitle>
      <p className="text-sm text-fg-muted">
        本页直接连接你选的节点，不经过论坛服务器。RPC 用来读链，Bundler
        用来提交操作；都可以换成你自己信任的。
      </p>
      <EndpointField label="RPC 节点" presets={rpcPresets()} value={rpc} onChange={setRpc} />
      <EndpointField
        label="Bundler（EntryPoint v0.7）"
        presets={bundlerPresets()}
        value={bundler}
        onChange={setBundler}
      />
      <ErrorLine error={error} />
      <Button disabled={busy || invalid} onClick={connect}>
        {busy ? <LoaderCircle size={18} className="animate-spin" aria-hidden /> : null}
        连接
      </Button>
    </Card>
  );
}

/** 第 2 步：两次 Face ID 找回钱包（Passkey 公钥 → 钱包地址）。 */
function FindStep({
  conn,
  onFound,
}: {
  conn: Connection;
  onFound: (c: WalletCandidate[]) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function find() {
    setBusy(true);
    setError("");
    try {
      const sdk = await loadEmergencySdk();
      onFound(await sdk.findWallets(conn, await sdk.recoverPasskey()));
    } catch (e) {
      setError(message(e, "没有找回钱包，请重试"));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Card className="space-y-3">
      <CardTitle>2. 用钱包 Passkey 找回钱包</CardTitle>
      <p className="text-sm text-fg-muted">
        需要验证两次 Face ID / 指纹：本页没有服务器告诉它你的钱包是哪个，要从两次签名里算出 Passkey
        的公钥，再算出钱包地址。这两次签名不会转走任何钱。
      </p>
      <ErrorLine error={error} />
      <Button disabled={busy} onClick={find}>
        <KeyRound size={18} aria-hidden />
        {busy ? "等待 Face ID…" : "用 Passkey 找回钱包"}
      </Button>
    </Card>
  );
}

function BalanceList({ b }: { b: Balances }) {
  return (
    <dl className="grid grid-cols-2 gap-2 text-sm sm:grid-cols-4">
      <div>
        <dt className="text-fg-muted">USDT</dt>
        <dd data-testid="emergency-usdt">
          <Amount value={b.usdt} />
        </dd>
      </div>
      <div>
        <dt className="text-fg-muted">ETH（付 gas）</dt>
        <dd className="amount" data-testid="emergency-eth">
          {formatEther(b.eth)}
        </dd>
      </div>
      <div>
        <dt className="text-fg-muted">担保合约可提取</dt>
        <dd>
          <Amount value={b.credit.escrow} />
        </dd>
      </div>
      <div>
        <dt className="text-fg-muted">分期合约可提取</dt>
        <dd>
          <Amount value={b.credit.stream} />
        </dd>
      </div>
    </dl>
  );
}

type Tab = "transfer" | "order" | "withdraw";

const TABS: { id: Tab; label: string }[] = [
  { id: "transfer", label: "转出 USDT" },
  { id: "order", label: "订单超时处理" },
  { id: "withdraw", label: "提取合约余额" },
];

const MARKET_LABEL: Record<Market, string> = { escrow: "一次性担保", stream: "分期担保" };
const marketAddress = (m: Market) =>
  m === "escrow" ? chain.contracts.Escrow : chain.contracts.StreamEscrow;

interface Draft {
  op: EmergencyOp;
  /** 确认页标题、金额、收款方 */
  action: string;
  amount: bigint;
  to: string;
  toLabel: string;
  note: ReactNode;
}

function TransferForm({
  wallet,
  balance,
  onDraft,
}: {
  wallet: EmergencyWallet;
  balance: bigint;
  onDraft: (d: Draft | null) => void;
}) {
  const [raw, setRaw] = useState("");
  const [to, setTo] = useState("");
  const [tail, setTail] = useState("");
  const [sure, setSure] = useState(false);
  const amount = parseUsdt(raw);
  const amountError = raw.trim() ? withdrawError(amount, balance, 0n) : null;
  const dest = checkAddress(to, {
    self: wallet.address,
    contracts: Object.values(chain.contracts),
  });
  const addressError = to.trim() && !dest.ok ? dest.error : null;
  // 强确认：再手打一遍地址最后 6 位，并勾选确认网络
  const tailOk = dest.ok && tail.trim().toLowerCase() === dest.address.slice(-6).toLowerCase();
  const ready = dest.ok && amount !== null && !amountError && amount > 0n && tailOk && sure;
  return (
    <div className="space-y-3">
      <Field label="收款地址（Arbitrum One）" error={addressError ?? undefined}>
        <Input
          value={to}
          onChange={(e) => setTo(e.target.value)}
          placeholder="0x…"
          spellCheck={false}
          autoComplete="off"
          className="amount font-mono text-sm"
        />
      </Field>
      <Field label="金额（USDT）" error={amountError ?? undefined}>
        <div className="flex gap-2">
          <Input
            inputMode="decimal"
            value={raw}
            onChange={(e) => setRaw(e.target.value)}
            placeholder="0.00"
            className="amount"
          />
          <Button
            variant="secondary"
            onClick={() => setRaw(formatUsdt(balance).replaceAll(",", ""))}
          >
            全部
          </Button>
        </div>
      </Field>
      <Field label="再输入一遍收款地址的最后 6 位">
        <Input
          value={tail}
          onChange={(e) => setTail(e.target.value)}
          maxLength={6}
          spellCheck={false}
          autoComplete="off"
          className="amount font-mono"
        />
      </Field>
      <label className="flex items-start gap-2 text-sm">
        <input
          type="checkbox"
          checked={sure}
          onChange={(e) => setSure(e.target.checked)}
          className="mt-1 accent-(--color-accent)"
        />
        <span>
          我确认收款地址是 <strong>Arbitrum One</strong> 网络上的地址（如交易所的 Arbitrum One
          充值地址）。转错地址或网络，资金无法找回。
        </span>
      </label>
      <Button
        disabled={!ready}
        onClick={() =>
          dest.ok &&
          amount !== null &&
          onDraft({
            op: { kind: "transfer", to: dest.address, amount },
            action: "转出 USDT",
            amount,
            to: dest.address,
            toLabel: "收款地址",
            note: "转出后无法撤回。请再核对一遍地址前后 6 位。",
          })
        }
      >
        下一步：估算 gas
      </Button>
    </div>
  );
}

function MarketPicker({ value, onChange }: { value: Market; onChange: (m: Market) => void }) {
  return (
    <fieldset className="flex flex-wrap gap-3 text-sm">
      <legend className="mb-1 font-medium">合约</legend>
      {(["escrow", "stream"] as const).map((m) => (
        <label key={m} className="flex items-center gap-1">
          <input
            type="radio"
            name="market"
            checked={value === m}
            onChange={() => onChange(m)}
            className="accent-(--color-accent)"
          />
          {MARKET_LABEL[m]}
        </label>
      ))}
    </fieldset>
  );
}

const ORDER_ID = /^\d{1,20}$/;

function OrderForm({ onDraft }: { onDraft: (d: Draft | null) => void }) {
  const [market, setMarket] = useState<Market>("escrow");
  const [id, setId] = useState("");
  const valid = ORDER_ID.test(id.trim());
  const draft = (kind: "claimTimeout" | "resolveTimeout"): Draft => ({
    op: { kind, market, orderId: BigInt(id.trim()) },
    action: kind === "claimTimeout" ? "超时取回 / 放款" : "争议超时结算",
    amount: 0n,
    to: marketAddress(market),
    toLabel: `${MARKET_LABEL[market]}合约`,
    note:
      kind === "claimTimeout"
        ? "期限已过才能执行：卖家逾期未发货 / 未开始 → 全额退给买家；已发货且确认期满 → 放款给卖家。钱只会转给订单的买卖双方。"
        : "争议 30 天未裁决才能执行，按默认规则结算（见订单规则），争议费全退。钱只会转给订单的买卖双方。",
  });
  return (
    <div className="space-y-3">
      <MarketPicker value={market} onChange={setMarket} />
      <Field
        label="链上订单号"
        hint="订单页里的链上订单号（不是站内订单 ID）；任何人都可以触发超时处理"
      >
        <Input
          value={id}
          onChange={(e) => setId(e.target.value)}
          inputMode="numeric"
          className="amount"
        />
      </Field>
      <div className="flex flex-wrap gap-2">
        <Button disabled={!valid} onClick={() => onDraft(draft("claimTimeout"))}>
          超时取回 / 放款（claimTimeout）
        </Button>
        <Button
          variant="secondary"
          disabled={!valid}
          onClick={() => onDraft(draft("resolveTimeout"))}
        >
          争议超时结算（resolveTimeout）
        </Button>
      </div>
    </div>
  );
}

function WithdrawForm({
  wallet,
  balances,
  onDraft,
}: {
  wallet: EmergencyWallet;
  balances: Balances;
  onDraft: (d: Draft | null) => void;
}) {
  const [market, setMarket] = useState<Market>("escrow");
  const [to, setTo] = useState<string>(wallet.address);
  const dest = checkAddress(to, { contracts: Object.values(chain.contracts) });
  const credit = balances.credit[market];
  return (
    <div className="space-y-3">
      <p className="text-sm text-fg-muted">
        担保合约放款、退款时如果转账失败，钱会留在合约里记在你名下，用这里取出。
      </p>
      <MarketPicker value={market} onChange={setMarket} />
      <p className="text-sm">
        可提取 <Amount value={credit} />
      </p>
      <Field label="转到" error={to.trim() && !dest.ok ? dest.error : undefined}>
        <Input
          value={to}
          onChange={(e) => setTo(e.target.value)}
          spellCheck={false}
          className="amount font-mono text-sm"
        />
      </Field>
      <Button
        disabled={credit === 0n || !dest.ok}
        onClick={() =>
          dest.ok &&
          onDraft({
            op: { kind: "withdraw", market, to: dest.address },
            action: "提取合约余额",
            amount: credit,
            to: dest.address,
            toLabel: "收款地址",
            note: `从${MARKET_LABEL[market]}合约取出记在你名下的余额。`,
          })
        }
      >
        下一步：估算 gas
      </Button>
    </div>
  );
}

/** 估算结果：要多少 ETH、还差多少、往哪转；够了才能签名。 */
function GasCheck({
  wallet,
  draft,
  eth,
  onSent,
}: {
  wallet: EmergencyWallet;
  draft: Draft;
  eth: bigint;
  onSent: (hash: string) => void;
}) {
  const [quote, setQuote] = useState<{ required: bigint } | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState(false);
  async function estimate() {
    setBusy(true);
    setError("");
    try {
      setQuote(await wallet.quote(draft.op));
    } catch (e) {
      setQuote(null);
      setError(message(e, "估算失败：这笔操作现在可能执行不了"));
    } finally {
      setBusy(false);
    }
  }
  const missing = quote ? topUp(quote.required, eth) : 0n;
  return (
    <div className="space-y-3 rounded-lg border border-border p-3" data-testid="gas-check">
      <p className="text-sm font-medium">{draft.action}</p>
      <Button size="sm" variant="secondary" disabled={busy} onClick={estimate}>
        {busy ? <LoaderCircle size={16} className="animate-spin" aria-hidden /> : null}
        {quote ? "重新估算" : "估算需要的 ETH"}
      </Button>
      <ErrorLine error={error} />
      {error ? (
        // Bundler 不支持估算时的状态覆盖、钱包又没有 ETH 时估不出来：照样告诉用户往哪转
        <p className="text-sm text-fg-muted">
          需要 ETH 时请转到钱包地址{" "}
          <span className="amount font-mono break-all">{wallet.address}</span>
        </p>
      ) : null}
      {quote ? (
        <div className="space-y-2 text-sm">
          <p data-testid="gas-required">
            最多需要 <span className="amount font-medium">{formatEther(quote.required)}</span> ETH
            （按当前 gas 价格的上限，用剩的退回钱包），钱包现有{" "}
            <span className="amount">{formatEther(eth)}</span> ETH。
          </p>
          {missing > 0n ? (
            <p role="alert" className="rounded-md bg-warning-soft p-3 text-warning">
              ETH 不够：请从交易所或其他钱包往下面这个地址转至少{" "}
              <strong className="amount">{formatEther(missing)}</strong> ETH（{chain.name}
              网络），到账后点「重新估算」。
              <span className="amount mt-1 block font-mono break-all" data-testid="fund-address">
                {wallet.address}
              </span>
            </p>
          ) : (
            <Button onClick={() => setOpen(true)}>签名并提交</Button>
          )}
        </div>
      ) : null}
      <SignConfirmDialog
        open={open}
        onOpenChange={setOpen}
        action={draft.action}
        amount={draft.amount.toString()}
        to={draft.to}
        toLabel={draft.toLabel}
        network={NETWORK_LABEL}
        rows={quote ? [{ label: "gas（ETH，上限）", value: formatEther(quote.required) }] : []}
        note={draft.note}
        gasNote="gas 由你钱包里的 ETH 支付，不经过平台"
        onConfirm={async () => {
          const hash = await wallet.send(draft.op);
          setOpen(false);
          onSent(hash);
        }}
      />
    </div>
  );
}

/** 第 3 步：钱包、余额与操作。 */
function WalletStep({ candidates }: { candidates: WalletCandidate[] }) {
  const [index, setIndex] = useState(candidates.length - 1);
  const picked = candidates[index] ?? candidates[0];
  const [balances, setBalances] = useState<Balances | undefined>(picked?.balances);
  const [tab, setTab] = useState<Tab>("transfer");
  const [draft, setDraft] = useState<Draft | null>(null);
  const [sent, setSent] = useState<string | null>(null);
  if (!picked || !balances) return null;
  const { wallet } = picked;
  async function refresh() {
    setBalances(await wallet.balances());
  }
  return (
    <Card className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <CardTitle>3. 你的钱包</CardTitle>
        <NetworkBadge label={NETWORK_LABEL} />
      </div>
      {candidates.length > 1 ? (
        <fieldset className="space-y-1 text-sm">
          <legend className="font-medium">这个 Passkey 有多个版本的钱包，选一个：</legend>
          {candidates.map((c, i) => (
            <label key={c.wallet.address} className="flex items-center gap-2">
              <input
                type="radio"
                checked={i === index}
                onChange={() => {
                  setIndex(i);
                  setBalances(c.balances);
                  setDraft(null);
                }}
              />
              <span className="amount font-mono break-all">{c.wallet.address}</span>（版本{" "}
              {c.wallet.version}）
            </label>
          ))}
        </fieldset>
      ) : null}
      <div>
        <p className="text-sm text-fg-muted">
          钱包地址{balances.deployed ? "" : "（还没部署，第一笔操作会顺带部署）"}
        </p>
        <p className="amount font-mono text-sm break-all" data-testid="emergency-address">
          {wallet.address}
        </p>
      </div>
      <BalanceList b={balances} />
      <Button size="sm" variant="secondary" onClick={refresh}>
        刷新余额
      </Button>
      <div className="flex flex-wrap gap-2" role="tablist">
        {TABS.map((t) => (
          <Button
            key={t.id}
            role="tab"
            aria-selected={tab === t.id}
            size="sm"
            variant={tab === t.id ? "primary" : "secondary"}
            onClick={() => {
              setTab(t.id);
              setDraft(null);
            }}
          >
            {t.label}
          </Button>
        ))}
      </div>
      {tab === "transfer" ? (
        <TransferForm wallet={wallet} balance={balances.usdt} onDraft={setDraft} />
      ) : tab === "order" ? (
        <OrderForm onDraft={setDraft} />
      ) : (
        <WithdrawForm wallet={wallet} balances={balances} onDraft={setDraft} />
      )}
      {draft ? (
        <GasCheck
          key={JSON.stringify(draft.op, (_k, v) => (typeof v === "bigint" ? v.toString() : v))}
          wallet={wallet}
          draft={draft}
          eth={balances.eth}
          onSent={(hash) => {
            setSent(hash);
            setDraft(null);
            void refresh();
          }}
        />
      ) : null}
      {sent ? (
        <p
          role="status"
          className="flex flex-wrap items-center gap-1 text-sm text-success"
          data-testid="emergency-done"
        >
          <CircleCheck size={16} aria-hidden />
          已上链，交易 <TxLink hash={sent} />
        </p>
      ) : null}
    </Card>
  );
}

/** 紧急取回：何时使用 → 选节点 → Passkey 找回钱包 → 估算 ETH → 签名提交。 */
export function EmergencyWithdraw() {
  const [conn, setConn] = useState<Connection | null>(null);
  const [candidates, setCandidates] = useState<WalletCandidate[] | null>(null);
  return (
    <div className="space-y-4">
      <Card className="space-y-2">
        <h1 className="text-xl font-semibold">紧急取回</h1>
        <div role="note" className="flex gap-2 rounded-lg bg-warning-soft p-3 text-sm text-warning">
          <TriangleAlert size={18} className="mt-0.5 shrink-0" aria-hidden />
          <p>
            只在论坛或平台代付长时间不可用时使用。平时请在钱包首页「提回交易所」，免 ETH、只收 0.05
            USDT 代付费。
          </p>
        </div>
        <ul className="list-disc space-y-1 pl-5 text-sm text-fg-muted">
          <li>钱一直在你自己的智能钱包和担保合约里，平台停摆也转不走、冻不住。</li>
          <li>
            本页不连论坛服务器：直接连公共 RPC 读链，经第三方 Bundler 提交，用你的钱包 Passkey
            签名。
          </li>
          <li>
            没有平台代付，gas 要用钱包里的 ETH 付（每笔通常不到几美分）；页面会算出要多少、转到哪。
          </li>
          <li>
            可以把 USDT 转到任意地址，也可以对订单触发超时取回 /
            争议超时结算、取出合约里记在你名下的余额。
          </li>
        </ul>
      </Card>
      <ConnectStep
        onConnected={(c) => {
          setConn(c);
          setCandidates(null);
        }}
      />
      {conn ? <FindStep conn={conn} onFound={setCandidates} /> : null}
      {candidates ? (
        <WalletStep key={candidates[0]?.wallet.address} candidates={candidates} />
      ) : null}
    </div>
  );
}
