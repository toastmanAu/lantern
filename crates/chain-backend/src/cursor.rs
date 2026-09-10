//! Paging cursors that cannot represent an exhausted scan.
//!
//! CKB's indexer returns `last_cursor: "0x"` once a scan is exhausted, and a
//! later `get_cells` with `after: "0x"` returns nothing **forever**, even for
//! a lock that holds cells. A pager that stores the terminal cursor therefore
//! goes permanently blind, silently. Verified against CKB testnet 2026-09-10.
//!
//! Two rules live here and nowhere else: an empty page yields no next cursor,
//! and a sentinel is not a cursor — and the private fields are what keep it
//! that way.

use crate::indexer::IndexerCell;

/// A resumable paging position.
///
/// Deliberately has no serde derives. A persisted cursor is a footgun: the
/// only safe way to revive one is through [`Cursor::resumable`], which
/// rejects the sentinel. If a future plan needs to persist paging state, it
/// must store the raw string and re-validate on read rather than deriving
/// `Deserialize` here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor(String);

impl Cursor {
    /// The sentinels a CKB node uses for "nothing further".
    const SENTINELS: [&'static str; 2] = ["0x", "0X"];

    /// Build a cursor, or `None` when the value cannot be resumed from.
    pub fn resumable(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || Self::SENTINELS.contains(&trimmed) {
            return None;
        }
        Some(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One page of a cell scan.
///
/// `next` is `None` whenever the scan cannot usefully continue, which is both
/// when the page came back empty and when the node handed back a sentinel.
#[derive(Debug, Clone)]
pub struct CellPage {
    cells: Vec<IndexerCell>,
    next: Option<Cursor>,
}

impl CellPage {
    /// The only way to build a page, and the single enforcement point for
    /// both cursor rules. The fields are private precisely so this cannot be
    /// sidestepped with a struct literal.
    pub fn new(cells: Vec<IndexerCell>, last_cursor: &str) -> Self {
        let next = if cells.is_empty() {
            None
        } else {
            Cursor::resumable(last_cursor)
        };
        Self { cells, next }
    }

    pub fn cells(&self) -> &[IndexerCell] {
        &self.cells
    }

    /// Take ownership of the rows, for callers accumulating across pages.
    #[must_use]
    pub fn into_cells(self) -> Vec<IndexerCell> {
        self.cells
    }

    pub const fn next(&self) -> Option<&Cursor> {
        self.next.as_ref()
    }

    /// Whether a pager driving this scan should stop.
    pub const fn is_exhausted(&self) -> bool {
        self.next.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::{CellPage, Cursor};
    use crate::indexer::sample_cell;

    // The exact byte string a real testnet node returned mid-scan.
    const REAL: &str = "0x409bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce801";

    #[test]
    fn sentinels_are_not_resumable() {
        assert!(
            Cursor::resumable("0x").is_none(),
            "the exhausted-scan sentinel"
        );
        assert!(Cursor::resumable("").is_none(), "an absent cursor");
        assert!(Cursor::resumable("   ").is_none(), "whitespace only");
        assert!(Cursor::resumable("0X").is_none(), "uppercase sentinel");
    }

    #[test]
    fn a_real_cursor_round_trips() {
        let c = Cursor::resumable(REAL).expect("a real cursor resumes");
        assert_eq!(c.as_str(), REAL);
    }

    #[test]
    fn an_empty_page_never_carries_a_next_cursor() {
        // Even if a node handed back something cursor-shaped on an empty page,
        // there is nothing left to fetch and storing it risks the poison.
        let page = CellPage::new(Vec::new(), REAL);
        assert!(page.cells().is_empty());
        assert!(page.next().is_none(), "empty page must not resume");
        assert!(page.is_exhausted());
    }

    #[test]
    fn a_full_page_carries_its_cursor() {
        let page = CellPage::new(vec![sample_cell()], REAL);
        assert_eq!(page.cells().len(), 1);
        assert_eq!(page.next().map(Cursor::as_str), Some(REAL));
        assert!(!page.is_exhausted());
    }

    #[test]
    fn the_live_exhaustion_sequence_terminates() {
        // Replays what testnet actually does: a full page, then an empty page
        // whose last_cursor is "0x". A pager driven by `next` must stop, and
        // must not have retained anything to feed back in.
        let page1 = CellPage::new(vec![sample_cell()], REAL);
        assert!(page1.next().is_some(), "first page continues");
        let page2 = CellPage::new(Vec::new(), "0x");
        assert!(page2.next().is_none(), "exhausted scan stops");
        assert!(page2.is_exhausted());
    }

    #[test]
    fn a_non_empty_page_with_a_sentinel_cursor_also_stops() {
        // Defence in depth: if a node ever returns rows plus "0x", resuming
        // from "0x" would return nothing, so treat it as exhausted.
        let page = CellPage::new(vec![sample_cell()], "0x");
        assert_eq!(page.cells().len(), 1, "rows are still delivered");
        assert!(page.next().is_none(), "but the scan does not continue");
    }

    #[test]
    fn a_page_can_only_be_built_through_new() {
        // If the fields were public a caller could pair an empty page with a
        // live cursor and resurrect the poison. They are private, so the only
        // route is `new`, which refuses. This test documents the reason;
        // making it fail requires re-publishing the fields, which will not
        // compile against these accessors.
        let page = CellPage::new(Vec::new(), REAL);
        assert!(page.next().is_none());
        assert!(page.cells().is_empty());
        assert!(page.into_cells().is_empty());
    }
}
