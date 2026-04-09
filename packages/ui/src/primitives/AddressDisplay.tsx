import type { ReactNode } from "react";

/**
 * AddressDisplay — locked security primitive.
 *
 * Renders a CKB address with safe truncation and an optional copy affordance.
 * NEVER render wallet addresses with a raw <span> or <code>; always use this
 * primitive so truncation, direction, and copy semantics stay consistent
 * across the app.
 *
 * Rules:
 *   - Full address must be available to assistive tech via aria-label.
 *   - Truncation is head+tail, never head-only (prevents homograph swaps at
 *     the tail going unnoticed).
 *   - `onCopy` receives the FULL address, not the truncated form.
 */
export interface AddressDisplayProps {
  address: string;
  /** Characters to show at the head before the ellipsis. Default 10. */
  head?: number;
  /** Characters to show at the tail after the ellipsis. Default 8. */
  tail?: number;
  /** Show a copy button. */
  copyable?: boolean;
  onCopy?: (fullAddress: string) => void;
  /** Optional label, e.g. "Receive address". */
  label?: ReactNode;
}

export function AddressDisplay({
  address,
  head = 10,
  tail = 8,
  copyable = false,
  onCopy,
  label,
}: AddressDisplayProps) {
  const shouldTruncate = address.length > head + tail + 1;
  const display = shouldTruncate
    ? `${address.slice(0, head)}…${address.slice(-tail)}`
    : address;

  return (
    <div data-lantern-address="" aria-label={address}>
      {label ? <span data-slot="label">{label}</span> : null}
      <span data-slot="value" title={address}>
        {display}
      </span>
      {copyable ? (
        <button
          type="button"
          data-slot="copy"
          aria-label="Copy address"
          onClick={() => onCopy?.(address)}
        >
          Copy
        </button>
      ) : null}
    </div>
  );
}
