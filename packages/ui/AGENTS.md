# @lantern/ui — Agent Composition Rules

This package is the **only** approved source of visual primitives for the Lantern
wallet. Agents (Claude Code, Figma MCP, code-gen tools) implementing screens
MUST follow these rules. Violations should be flagged in code review.

## Core rules

1. **Compose, don't invent.** Build screens by composing exported primitives
   from `@lantern/ui`. Do NOT create new primitives inline inside
   `apps/desktop/src/features/**`. New primitives require an explicit request
   and land here, with tests.

2. **Tokens only — no magic values.** All colors, spacing, radii, font sizes,
   and shadows MUST reference CSS variables defined in `src/tokens.css`.
   No hex codes, no `px` literals for spacing, no ad-hoc `rgba(...)`.
   If a token is missing, add it to `tokens.css` first, then use it.

3. **Presentational, not transactional.** Primitives in this package must be
   pure presentational components. They never:
   - import from `@tauri-apps/api`
   - call IPC / invoke Rust commands
   - read or mutate wallet state (zustand stores)
   - touch `window`, `localStorage`, or any browser global at module scope
   Business logic lives in `apps/desktop/src/features/<feature>/`.

4. **Accessibility is not optional.** Every interactive primitive needs a
   keyboard path, a visible focus ring (use the `--focus-ring` token), and
   correct ARIA. Wallet users include people with screen readers and people
   confirming 5-figure transactions — both deserve legible affordances.

5. **Security-critical primitives are locked.** `AddressDisplay`,
   `AmountInput`, `TxReviewPanel`, `SeedPhraseDisplay`, and `WarningBanner`
   are considered security surfaces. Changes to their visual behavior
   (truncation, copy, reveal, confirmation) require human review — flag
   the diff explicitly in the PR description.

## Layout rules (locked)

These are hard constraints for every screen in `apps/desktop/src/features/**`.
Generated screens that violate these MUST be regenerated, not patched.

- **Desktop-first, ~1280×800 baseline.** Lantern is a Tauri desktop wallet, not
  a mobile web app. Use the horizontal width.
- **Sidebar nav rail + two-pane main area** is the default shell. Left rail:
  Send / Receive / Channels / Settings. Main area: form pane (left) + live tx
  preview / account context pane (right). The preview pane builds trust by
  showing the user what they're about to sign as they type.
- **NO centered narrow column.** `max-w-lg`, `max-w-md`, `max-w-sm` on a top-
  level screen container is forbidden — it looks like a mobile emulator on a
  desktop window. The only exception is genuinely single-action surfaces
  (unlock prompt, seed reveal modal).
- **NO vertical scroll on standard screens.** Every screen must fit the
  viewport. If content overflows, the design is wrong — split into tabs,
  collapse sections, or rethink the screen. Scroll is only acceptable for
  explicit list surfaces (tx history, address book).
- **Accent color is Nervos green** (`--lantern-color-accent`, `#3cc68a`).
  Never reach for blue or cyan, even as a placeholder.

## Composition checklist for generated screens

Before committing an agent-generated screen, verify:

- [ ] Only imports from `@lantern/ui`, `react`, `@tanstack/react-router`,
      `@tanstack/react-query`, and local feature files
- [ ] No hex colors, no inline `style={{ padding: '12px' }}`
- [ ] All interactive elements reachable by Tab, with visible focus
- [ ] Loading, error, empty, and disabled states rendered
- [ ] No direct Tauri IPC — calls go through a feature hook
- [ ] Security-critical values (addresses, amounts, fees) use the locked
      primitives, not raw `<span>` / `<input>`

## When Figma MCP generates code

When driving this package from Figma frames via `figma-implement-design`:

- Map Figma variables to existing tokens in `tokens.css`. Do not duplicate.
- If Figma introduces a new token, add it to `tokens.css` with a comment
  pointing to the Figma variable name.
- Component names in Figma should match the exported primitive name
  (`Button`, `AmountInput`, ...). Variants map to props, not new components.

## Out of scope for this package

- Routing, data fetching, state management
- Tauri-specific glue code
- Wallet domain logic (key derivation, signing, fee calc)
- Network / RPC calls

If an agent finds itself wanting any of the above inside `@lantern/ui`,
**stop and escalate** — it belongs in `apps/desktop` or the Rust core.
