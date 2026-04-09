import type { ReactNode } from "react";

/**
 * Sanitize a raw amount string into a canonical numeric form.
 *
 * Policy (Option B — accept + normalize, English locale only):
 *   - Commas are ALWAYS stripped. They are never reinterpreted as a decimal
 *     mark, even when typed alone — that surprised users in testing
 *     ("I typed a comma and it became a dot"). Letters and stray symbols
 *     are dropped silently; commas now behave the same way for consistency.
 *     Pasting "1,234.56" still cleanly becomes "1234.56".
 *   - Strip any character that is not a digit or a decimal point.
 *   - Collapse multiple decimal points to the first one.
 *   - Truncate the fractional part to `decimals` digits (no rounding —
 *     rounding an amount the user typed would be a wallet footgun).
 *   - Preserve a trailing decimal point (e.g. "12.") so typing feels natural.
 *   - Empty string stays empty — never coerce to "0".
 */
export function sanitizeAmount(raw: string, decimals: number): string {
  if (raw === "") return "";

  // Always strip commas. The wallet is English-locale; commas are thousands
  // separators on paste and noise on type. Never a decimal mark.
  let normalized = raw.replace(/,/g, "");

  // Keep only digits and dots.
  normalized = normalized.replace(/[^\d.]/g, "");

  // Collapse to at most one decimal point — keep the first one.
  const firstDot = normalized.indexOf(".");
  if (firstDot !== -1) {
    normalized =
      normalized.slice(0, firstDot + 1) +
      normalized.slice(firstDot + 1).replace(/\./g, "");
  }

  // Truncate the fractional part to `decimals` digits.
  if (firstDot !== -1 && decimals >= 0) {
    const [intPart = "", fracPart = ""] = normalized.split(".");
    normalized =
      decimals === 0
        ? intPart
        : `${intPart}.${fracPart.slice(0, decimals)}`;
  }

  return normalized;
}

/**
 * AmountInput — locked security primitive.
 *
 * Renders a numeric input for CKB / Fibt / token amounts with an explicit
 * unit label. Wallet amounts MUST go through this component so that unit
 * display, decimal handling, and max-balance affordances are consistent
 * across Send, Swap, Fiber channel, and any future flows.
 *
 * PHILL — this is where I'm leaving you the meaningful decision. See the
 * TODO block inside `handleChange`. It's ~8 lines and it shapes every
 * amount-entry surface in the wallet.
 */
export interface AmountInputProps {
  value: string;
  onChange: (next: string) => void;
  /** Unit label shown inside the input, e.g. "CKB", "Fibt". */
  unit: string;
  /** Max decimals the unit supports. CKB = 8. */
  decimals: number;
  /** Optional balance for the "Max" button. */
  balance?: string;
  onMax?: () => void;
  disabled?: boolean;
  error?: ReactNode;
  placeholder?: string;
}

export function AmountInput({
  value,
  onChange,
  unit,
  decimals,
  balance,
  onMax,
  disabled,
  error,
  placeholder = "0.0",
}: AmountInputProps) {
  function handleChange(raw: string) {
    onChange(sanitizeAmount(raw, decimals));
  }

  return (
    <div
      data-lantern-amount=""
      data-disabled={disabled ? "" : undefined}
      data-invalid={error ? "" : undefined}
    >
      <div data-slot="row">
        <input
          data-slot="input"
          inputMode="decimal"
          autoComplete="off"
          spellCheck={false}
          placeholder={placeholder}
          value={value}
          disabled={disabled}
          onChange={(e) => handleChange(e.target.value)}
          aria-invalid={error ? true : undefined}
        />
        <span data-slot="unit">{unit}</span>
        {onMax ? (
          <button
            type="button"
            data-slot="max"
            onClick={onMax}
            disabled={disabled}
          >
            Max
          </button>
        ) : null}
      </div>
      {balance ? (
        <div data-slot="balance">
          Balance: {balance} {unit}
        </div>
      ) : null}
      {error ? (
        <div data-slot="error" role="alert">
          {error}
        </div>
      ) : null}
      {/* decimals is passed through for the sanitization policy above */}
      <span data-slot="decimals" hidden>
        {decimals}
      </span>
    </div>
  );
}
