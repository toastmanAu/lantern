import { useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import {
  AddressInput,
  AmountInput,
  Button,
  TxReviewPanel,
  WarningBanner,
  type TxReviewMetaRow,
} from "@lantern/ui";
import "./index.css";

export const Route = createFileRoute("/")({
  component: SendSandboxPage,
});

/**
 * Sandbox Send screen. NOT the production SendScreen — that lands in
 * apps/desktop/src/features/send/ once feature routes come online. This
 * route exists to:
 *
 *   1. Visually verify the harvested @lantern/ui primitives render in the
 *      Tauri shell against the locked Nervos green tokens.
 *   2. Validate the sidebar + form-pane + tx-review-pane layout from
 *      packages/ui/AGENTS.md fits a desktop window without scroll.
 *   3. Wire form state into TxReviewPanel so the right pane updates as the
 *      user types — proving the locked "show what you'll sign" pattern
 *      works in practice.
 */

const FROM_ADDRESS =
  "ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsq2g7h8ndnn";
const FEE_FIBT = "0.00100000";
const FEE_NUM = 0.001;

function SendSandboxPage() {
  const [recipient, setRecipient] = useState("");
  const [amount, setAmount] = useState("");

  // The TxReviewPanel takes pre-formatted strings only — caller does any
  // math. Empty strings collapse to undefined so the panel hides those rows.
  const amountForReview = amount === "" ? undefined : amount;
  const totalForReview = (() => {
    if (amount === "") return undefined;
    const n = Number(amount);
    if (Number.isNaN(n)) return undefined;
    return (n + FEE_NUM).toFixed(8);
  })();

  const meta: TxReviewMetaRow[] = [
    { label: "Network", value: "CKB Testnet" },
    { label: "Memo", value: "—" },
  ];

  return (
    <div className="lantern-shell">
      {/* ─── Nav rail ─── */}
      <nav className="lantern-rail" aria-label="Primary">
        <div className="lantern-rail-brand">
          <span className="lantern-rail-brand-dot" aria-hidden />
          Lantern
        </div>
        <ul className="lantern-rail-nav">
          <li>
            <button type="button" data-active>
              <span className="lantern-rail-glyph" aria-hidden>
                ↗
              </span>
              Send
            </button>
          </li>
          <li>
            <button type="button">
              <span className="lantern-rail-glyph" aria-hidden>
                ↘
              </span>
              Receive
            </button>
          </li>
          <li>
            <button type="button">
              <span className="lantern-rail-glyph" aria-hidden>
                ⟷
              </span>
              Channels
            </button>
          </li>
          <li>
            <button type="button">
              <span className="lantern-rail-glyph" aria-hidden>
                ⚙
              </span>
              Settings
            </button>
          </li>
        </ul>
        <div className="lantern-rail-footer">v0.0.4 · sandbox</div>
      </nav>

      {/* ─── Form pane ─── */}
      <section className="lantern-form-pane" aria-label="Send form">
        <header className="lantern-form-pane-header">
          <h1>Send Fibt</h1>
          <p>Enter recipient and amount.</p>
        </header>

        <WarningBanner severity="warning" title="Testnet mode">
          You are connected to the CKB testnet. Amounts are in Fibt and have
          no real value.
        </WarningBanner>

        <AddressInput
          label="Recipient address"
          value={recipient}
          onChange={setRecipient}
          placeholder="ckt1..."
        />

        <AmountInput
          value={amount}
          onChange={setAmount}
          unit="Fibt"
          decimals={8}
          balance="1234.56789012"
          onMax={() => setAmount("1234.56789012")}
        />

        <div className="lantern-form-actions">
          <Button
            variant="primary"
            size="md"
            disabled={recipient === "" || amount === ""}
          >
            Send
          </Button>
          <Button
            variant="secondary"
            size="md"
            onClick={() => {
              setRecipient("");
              setAmount("");
            }}
          >
            Cancel
          </Button>
        </div>
      </section>

      {/* ─── Review pane ─── */}
      <aside className="lantern-review-pane" aria-label="Transaction review">
        <TxReviewPanel
          kind="send"
          network="testnet"
          from={FROM_ADDRESS}
          to={recipient || undefined}
          amount={amountForReview}
          amountUnit="Fibt"
          fee={FEE_FIBT}
          feeUnit="Fibt"
          total={totalForReview}
          totalUnit="Fibt"
          meta={meta}
        />
      </aside>
    </div>
  );
}
