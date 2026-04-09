import type { ReactNode } from "react";

/**
 * AddressInput — editable counterpart to AddressDisplay.
 *
 * Use this for any field where the user TYPES or PASTES a CKB address
 * (recipient on Send, channel counterparty, etc.). For RENDERING an address
 * the user is not editing — receive address, tx history rows, balance card —
 * use AddressDisplay instead.
 *
 * Rules:
 *   - Mono font (so paste of long addresses is legible at a glance).
 *   - Trim whitespace on input — pasted addresses often carry leading or
 *     trailing spaces from copy buffers, and CKB addresses never contain
 *     internal whitespace.
 *   - Error renders as a soft helper line + border tint, NEVER as a full red
 *     fill on the input itself. The user must always be able to read what
 *     they typed.
 *   - This primitive does NOT validate address format — that's a feature
 *     concern (network-specific prefix checks live in the Send hook).
 */
export interface AddressInputProps {
  value: string;
  onChange: (next: string) => void;
  label?: ReactNode;
  placeholder?: string;
  error?: ReactNode;
  disabled?: boolean;
}

export function AddressInput({
  value,
  onChange,
  label,
  placeholder = "ckt1...",
  error,
  disabled,
}: AddressInputProps) {
  return (
    <div
      data-lantern-address-input=""
      data-disabled={disabled ? "" : undefined}
      data-invalid={error ? "" : undefined}
    >
      {label ? <span data-slot="label">{label}</span> : null}
      <input
        data-slot="input"
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value.trim())}
        placeholder={placeholder}
        disabled={disabled}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={error ? true : undefined}
      />
      {error ? (
        <div data-slot="error" role="alert">
          {error}
        </div>
      ) : null}
    </div>
  );
}
