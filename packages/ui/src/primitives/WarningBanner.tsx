import type { ReactNode } from "react";

/**
 * WarningBanner — locked security primitive.
 *
 * Used for signing warnings, network mismatch alerts, testnet indicators,
 * fee anomalies, and any other state the user must acknowledge before
 * proceeding. Severity MUST match the actual risk — do not downgrade
 * critical warnings to `info` for aesthetic reasons.
 */
export type WarningSeverity = "info" | "warning" | "danger";

export interface WarningBannerProps {
  severity: WarningSeverity;
  title: ReactNode;
  children?: ReactNode;
  /** Optional dismiss handler. Critical warnings should be non-dismissable. */
  onDismiss?: () => void;
}

export function WarningBanner({
  severity,
  title,
  children,
  onDismiss,
}: WarningBannerProps) {
  return (
    <div
      data-lantern-banner=""
      data-severity={severity}
      role={severity === "danger" ? "alert" : "status"}
    >
      <div data-slot="title">{title}</div>
      {children ? <div data-slot="body">{children}</div> : null}
      {onDismiss && severity !== "danger" ? (
        <button
          type="button"
          data-slot="dismiss"
          aria-label="Dismiss"
          onClick={onDismiss}
        >
          ×
        </button>
      ) : null}
    </div>
  );
}
