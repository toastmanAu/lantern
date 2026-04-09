import type { ReactNode } from "react";

/**
 * TxReviewPanel — locked security primitive.
 *
 * Renders a structured "what you are about to sign" card. This is the right
 * pane in the sidebar + two-pane wallet layout, sitting next to whichever
 * action form is active (Send / Swap / Channel open / Sign message). Its job
 * is to show the user EXACTLY what their signature will authorise — every
 * field shown here MUST match the bytes that will hit the signer.
 *
 * Rules (locked):
 *   - Pure presentational. No state, no IPC, no formatting math. The caller
 *     is responsible for sanitising amounts, computing totals, and resolving
 *     fees BEFORE passing them in. The panel renders strings, never numbers.
 *   - Addresses always render through `AddressDisplay` so truncation, copy
 *     semantics, and full-address aria-labels stay consistent.
 *   - Network mismatch is communicated visually via the `network` prop —
 *     never via `warnings` text alone, because users skim warning text.
 *   - The action label (`kind`) sets the panel's intent badge. Do not pass
 *     freeform strings — that's how phishing-style typos slip through.
 *   - This primitive renders NO action buttons. The Send button, the Cancel
 *     button, the Confirm button — those live in the form pane next to the
 *     panel, not inside it. Confirmation surfaces and review surfaces are
 *     deliberately kept separate.
 */
export type TxReviewKind =
  | "send"
  | "swap"
  | "channel-open"
  | "channel-close"
  | "sign-message";

export type TxReviewNetwork = "mainnet" | "testnet";

export interface TxReviewMetaRow {
  label: ReactNode;
  value: ReactNode;
  /** Visual emphasis. Use sparingly. */
  emphasis?: "default" | "warning" | "danger";
}

export interface TxReviewPanelProps {
  kind: TxReviewKind;
  network: TxReviewNetwork;
  /** Address of the sender / origin account. */
  from?: string;
  /** Address of the recipient / counterparty. */
  to?: string;
  /** Principal amount as a pre-sanitised display string (no math here). */
  amount?: string;
  amountUnit?: string;
  /** Network fee, pre-formatted. */
  fee?: string;
  feeUnit?: string;
  /** Total = amount + fee. Caller computes; panel renders. */
  total?: string;
  totalUnit?: string;
  /** Optional key-value rows below the amounts (memo, channel id, etc.). */
  meta?: TxReviewMetaRow[];
  /** Inline warning slot — pass <WarningBanner/> elements here. */
  warnings?: ReactNode;
}

const KIND_LABELS: Record<TxReviewKind, string> = {
  send: "Send transaction",
  swap: "Swap",
  "channel-open": "Open channel",
  "channel-close": "Close channel",
  "sign-message": "Sign message",
};

export function TxReviewPanel({
  kind,
  network,
  from,
  to,
  amount,
  amountUnit,
  fee,
  feeUnit,
  total,
  totalUnit,
  meta,
  warnings,
}: TxReviewPanelProps) {
  return (
    <section
      data-lantern-tx-review=""
      data-network={network}
      aria-label={`${KIND_LABELS[kind]} review`}
    >
      <header data-slot="header">
        <span data-slot="kind">{KIND_LABELS[kind]}</span>
        <span data-slot="network" data-network={network}>
          {network === "mainnet" ? "Mainnet" : "Testnet"}
        </span>
      </header>

      {from ? (
        <div data-slot="party" data-party-role="from">
          <span data-slot="party-label">From</span>
          <span data-slot="party-value" title={from} aria-label={from}>
            {truncate(from)}
          </span>
        </div>
      ) : null}

      {to ? (
        <div data-slot="party" data-party-role="to">
          <span data-slot="party-label">To</span>
          <span data-slot="party-value" title={to} aria-label={to}>
            {truncate(to)}
          </span>
        </div>
      ) : null}

      {warnings ? <div data-slot="warnings">{warnings}</div> : null}

      {amount || fee || total ? (
        <dl data-slot="amounts">
          {amount ? (
            <div data-slot="amount-row" data-amount-role="principal">
              <dt>Amount</dt>
              <dd>
                <span data-slot="number">{amount}</span>
                {amountUnit ? <span data-slot="unit">{amountUnit}</span> : null}
              </dd>
            </div>
          ) : null}
          {fee ? (
            <div data-slot="amount-row" data-amount-role="fee">
              <dt>Network fee</dt>
              <dd>
                <span data-slot="number">{fee}</span>
                {feeUnit ? <span data-slot="unit">{feeUnit}</span> : null}
              </dd>
            </div>
          ) : null}
          {total ? (
            <div data-slot="amount-row" data-amount-role="total">
              <dt>Total</dt>
              <dd>
                <span data-slot="number">{total}</span>
                {totalUnit ? <span data-slot="unit">{totalUnit}</span> : null}
              </dd>
            </div>
          ) : null}
        </dl>
      ) : null}

      {meta && meta.length > 0 ? (
        <dl data-slot="meta">
          {meta.map((row, i) => (
            <div
              key={i}
              data-slot="meta-row"
              data-emphasis={row.emphasis ?? "default"}
            >
              <dt>{row.label}</dt>
              <dd>{row.value}</dd>
            </div>
          ))}
        </dl>
      ) : null}
    </section>
  );
}

/**
 * Internal head+tail truncate. Mirrors AddressDisplay's policy so the review
 * panel renders addresses identically to the form pane.
 */
function truncate(address: string, head = 10, tail = 8): string {
  if (address.length <= head + tail + 1) return address;
  return `${address.slice(0, head)}…${address.slice(-tail)}`;
}
