//! Rust port of the deletion-diff computation inside
//! `New-RecommenderRetirementPacket.ps1`: a delete-only, line-numbered patch
//! of the two bounded blocks that own the retired inline recommender in
//! `run-code-intel.ps1`. See `recommender_retirement_packet`'s module doc
//! for the overall port's scope.

use crate::recommender_retirement_shared::find_bounded_block;

pub(crate) struct DeleteHunk {
    pub(crate) deleted_lines: Vec<String>,
    pub(crate) old_start: usize,
    pub(crate) old_lines: usize,
    pub(crate) new_start: usize,
}

pub(crate) fn compute_delete_hunks(
    base_text: &str,
    patterns: &[(&str, &str)],
) -> Result<Vec<DeleteHunk>, String> {
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(patterns.len());
    for (start_marker, end_marker) in patterns {
        spans.push(find_bounded_block(base_text, start_marker, end_marker)?);
    }
    spans.sort_by_key(|span| span.0);
    let mut deleted_before = 0usize;
    let mut hunks = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        let deleted_lines: Vec<String> = base_text[start..end]
            .split('\n')
            .map(str::to_string)
            .collect();
        let old_start = base_text[..start].matches('\n').count() + 1;
        let old_lines = deleted_lines.len();
        hunks.push(DeleteHunk {
            deleted_lines,
            old_start,
            old_lines,
            new_start: old_start - deleted_before,
        });
        deleted_before += old_lines;
    }
    Ok(hunks)
}

pub(crate) fn build_result_text(base_text: &str, hunks: &[DeleteHunk]) -> String {
    base_text
        .split('\n')
        .enumerate()
        .filter(|(index, _)| {
            let line = index + 1;
            !hunks
                .iter()
                .any(|hunk| line >= hunk.old_start && line < hunk.old_start + hunk.old_lines)
        })
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_hunks_are_ordered_by_position_regardless_of_pattern_order() {
        let base = "line1\nSECOND block\nx\nEND2\nline5\nFIRST block\ny\nEND1\nline9";
        let hunks = compute_delete_hunks(
            base,
            &[("FIRST block", "\nEND1"), ("SECOND block", "\nEND2")],
        )
        .unwrap();
        assert_eq!(hunks.len(), 2);
        assert!(
            hunks[0].old_start < hunks[1].old_start,
            "hunks must be position-ordered, not pattern-ordered"
        );
        assert_eq!(hunks[0].old_start, 2);
        assert_eq!(hunks[0].old_lines, 2);
        assert_eq!(hunks[1].new_start, hunks[1].old_start - hunks[0].old_lines);
    }

    #[test]
    fn result_text_drops_exactly_the_deleted_line_ranges() {
        let base = "keep1\nSTART\nmid\nEND_MARK\nkeep2";
        let hunks = compute_delete_hunks(base, &[("START", "\nEND_MARK")]).unwrap();
        let result = build_result_text(base, &hunks);
        assert_eq!(result, "keep1\nEND_MARK\nkeep2");
    }
}
