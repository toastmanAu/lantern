import type { ButtonHTMLAttributes, ReactNode, Ref } from "react";

/**
 * Button — the only approved button primitive for Lantern.
 *
 * Variants:
 *   - primary:   main call-to-action (Send, Confirm, Sign)
 *   - secondary: supporting actions (Cancel, Back)
 *   - danger:    destructive confirmations (Reset wallet, Remove account)
 *   - ghost:     low-emphasis inline actions
 *
 * Security note: the `danger` variant MUST be used for any action that is
 * destructive or irreversible. Do not reuse `primary` styling for such flows.
 */
export type ButtonVariant = "primary" | "secondary" | "danger" | "ghost";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "className"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  leadingIcon?: ReactNode;
  trailingIcon?: ReactNode;
  ref?: Ref<HTMLButtonElement>;
}

export function Button({
  variant = "primary",
  size = "md",
  loading = false,
  disabled,
  leadingIcon,
  trailingIcon,
  children,
  ref,
  ...rest
}: ButtonProps) {
  return (
    <button
      ref={ref}
      data-lantern-button=""
      data-variant={variant}
      data-size={size}
      data-loading={loading ? "" : undefined}
      disabled={disabled || loading}
      {...rest}
    >
      {leadingIcon ? <span data-slot="leading">{leadingIcon}</span> : null}
      <span data-slot="label">{children}</span>
      {trailingIcon ? <span data-slot="trailing">{trailingIcon}</span> : null}
    </button>
  );
}
