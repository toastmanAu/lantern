/**
 * Tests for sanitizeAmount — Option B (accept + normalize) policy.
 *
 * Run via the workspace test runner once one is wired in (plan 1f).
 * Uses node:test so it has zero dependencies right now.
 */
import { describe, it } from "node:test";
import { strict as assert } from "node:assert";
import { sanitizeAmount } from "./AmountInput.js";

describe("sanitizeAmount", () => {
  const CKB = 8;

  it("passes through a clean value", () => {
    assert.equal(sanitizeAmount("123.456", CKB), "123.456");
  });

  it("keeps empty string empty", () => {
    assert.equal(sanitizeAmount("", CKB), "");
  });

  it("preserves a trailing decimal so typing feels natural", () => {
    assert.equal(sanitizeAmount("12.", CKB), "12.");
  });

  it("always strips commas — never reinterprets as decimal mark", () => {
    // Typing or pasting a lone comma should drop it silently, the same way
    // letters and other non-numeric input are dropped. The previous policy
    // converted "1,5" → "1.5" which surprised users.
    assert.equal(sanitizeAmount("1,5", CKB), "15");
    assert.equal(sanitizeAmount(",", CKB), "");
  });

  it("strips commas used as thousands separators", () => {
    assert.equal(sanitizeAmount("1,000,000.25", CKB), "1000000.25");
  });

  it("drops non-numeric garbage", () => {
    assert.equal(sanitizeAmount("12abc.3x4", CKB), "12.34");
  });

  it("collapses multiple decimal points to the first", () => {
    assert.equal(sanitizeAmount("1.2.3.4", CKB), "1.234");
  });

  it("truncates — never rounds — excess fraction digits", () => {
    // 9 digits of fraction, CKB supports 8 — the last digit MUST be dropped,
    // not rounded. Rounding a user-entered amount would be a wallet footgun.
    assert.equal(sanitizeAmount("0.123456789", CKB), "0.12345678");
  });

  it("handles a zero-decimals unit by dropping the fractional part", () => {
    assert.equal(sanitizeAmount("42.99", 0), "42");
  });

  it("does not coerce an empty fraction to zero", () => {
    assert.equal(sanitizeAmount(".", CKB), ".");
  });

  it("rejects stray signs and letters", () => {
    assert.equal(sanitizeAmount("-12", CKB), "12");
    assert.equal(sanitizeAmount("+0.5", CKB), "0.5");
    assert.equal(sanitizeAmount("NaN", CKB), "");
  });
});
