/**
 * @lantern/ui — shared React primitives for the Lantern wallet.
 *
 * Agents: read AGENTS.md in this package before generating screens or
 * adding new primitives. Composition rules are enforced in review.
 */

import "./tokens.css";
import "./primitives/Button.css";
import "./primitives/AddressDisplay.css";
import "./primitives/AddressInput.css";
import "./primitives/AmountInput.css";
import "./primitives/WarningBanner.css";
import "./primitives/TxReviewPanel.css";

export { Button } from "./primitives/Button.js";
export type { ButtonProps, ButtonVariant, ButtonSize } from "./primitives/Button.js";

export { AddressDisplay } from "./primitives/AddressDisplay.js";
export type { AddressDisplayProps } from "./primitives/AddressDisplay.js";

export { AddressInput } from "./primitives/AddressInput.js";
export type { AddressInputProps } from "./primitives/AddressInput.js";

export { AmountInput, sanitizeAmount } from "./primitives/AmountInput.js";
export type { AmountInputProps } from "./primitives/AmountInput.js";

export { WarningBanner } from "./primitives/WarningBanner.js";
export type { WarningBannerProps, WarningSeverity } from "./primitives/WarningBanner.js";

export { TxReviewPanel } from "./primitives/TxReviewPanel.js";
export type {
  TxReviewPanelProps,
  TxReviewKind,
  TxReviewNetwork,
  TxReviewMetaRow,
} from "./primitives/TxReviewPanel.js";

export const VERSION = "0.0.4";
