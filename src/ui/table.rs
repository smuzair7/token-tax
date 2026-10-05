use crate::pricing::Row;

/// Everything the renderers need, resolved before any terminal setup.
pub struct TableData<'a> {
    pub rows: &'a [Row],
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub byte_count: usize,
    pub encoding: &'a str,
    pub source: &'a str,
    pub source_is_stale: bool,
    /// Why the live fetch failed, shown when `source_is_stale`.
    pub fetch_error: Option<&'a str>,
    pub elapsed_ms: f64,
}

impl<'a> TableData<'a> {
    /// Largest total cost across priced rows, used to scale the bar column.
    ///
    /// `None` when nothing resolved, so the caller can skip bar rendering
    /// instead of scaling against a meaningless zero.
    pub fn max_total(&self) -> Option<f64> {
        self.rows
            .iter()
            .filter_map(|r| r.price.as_ref())
            .map(|p| p.total_cost(self.input_tokens, self.output_tokens))
            .fold(None, |acc: Option<f64>, v| {
                Some(acc.map_or(v, |a| a.max(v)))
            })
    }

    /// Total for a priced row at the assumed token counts.
    pub fn total_for(&self, row: &Row) -> Option<f64> {
        row.price
            .as_ref()
            .map(|p| p.total_cost(self.input_tokens, self.output_tokens))
    }

    /// Index of the cheapest priced row, for the bold/"cheapest" marker.
    pub fn cheapest_index(&self) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for (i, row) in self.rows.iter().enumerate() {
            if let Some(total) = self.total_for(row) {
                if best.is_none_or(|(_, b)| total < b) {
                    best = Some((i, total));
                }
            }
        }
        best.map(|(i, _)| i)
    }

    /// Rows whose input plus output exceeds the model's context window.
    pub fn overflowing_rows(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.price
                    .as_ref()
                    .is_some_and(|p| !p.fits(self.input_tokens, self.output_tokens))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Rows that failed to resolve.
    pub fn unknown_rows(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.is_unknown())
            .map(|(i, _)| i)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::ModelPrice;

    fn priced(id: &str, prompt: f64, completion: f64, ctx: Option<u64>) -> Row {
        Row {
            requested: id.to_string(),
            price: Some(ModelPrice {
                id: id.to_string(),
                prompt_per_token: prompt,
                completion_per_token: completion,
                context_length: ctx,
            }),
            suggestions: vec![],
        }
    }

    fn unknown(id: &str) -> Row {
        Row {
            requested: id.to_string(),
            price: None,
            suggestions: vec!["x/y".to_string()],
        }
    }

    fn data<'a>(rows: &'a [Row], out: u64) -> TableData<'a> {
        TableData {
            rows,
            input_tokens: 1000,
            output_tokens: out,
            byte_count: 4000,
            encoding: "cl100k_base",
            source: "live",
            source_is_stale: false,
            fetch_error: None,
            elapsed_ms: 1.0,
        }
    }

    #[test]
    fn max_total_tracks_the_deepest_row() {
        let rows = vec![
            priced("a", 3e-6, 1.5e-5, None),
            priced("b", 1e-8, 1e-8, None),
        ];
        let d = data(&rows, 1000);
        let max = d.max_total().unwrap();
        assert!((max - (3e-6 * 1000.0 + 1.5e-5 * 1000.0)).abs() < 1e-12);
    }

    #[test]
    fn max_total_is_none_when_nothing_resolved() {
        let rows = vec![unknown("a"), unknown("b")];
        assert_eq!(data(&rows, 10).max_total(), None);
    }

    #[test]
    fn cheapest_index_skips_unknown_rows() {
        let rows = vec![
            unknown("a"),
            priced("b", 1e-3, 1e-3, None),
            priced("c", 1e-2, 1e-2, None),
        ];
        assert_eq!(data(&rows, 100).cheapest_index(), Some(1));
    }

    #[test]
    fn cheapest_index_is_none_with_no_priced_rows() {
        let rows = vec![unknown("a")];
        assert_eq!(data(&rows, 10).cheapest_index(), None);
    }

    #[test]
    fn overflowing_rows_flag_context_overrun() {
        let rows = vec![
            priced("small", 1e-6, 1e-6, Some(1_000)),
            priced("big", 1e-6, 1e-6, Some(10_000_000)),
        ];
        // 1000 in + 5000 out = 6000, over the 1000-token window.
        assert_eq!(data(&rows, 5_000).overflowing_rows(), vec![0]);
    }

    #[test]
    fn unknown_context_length_is_never_an_overflow() {
        let rows = vec![priced("unknown-ctx", 1e-6, 1e-6, None)];
        assert!(data(&rows, u64::MAX).overflowing_rows().is_empty());
    }

    #[test]
    fn unknown_rows_lists_failures() {
        let rows = vec![priced("a", 1e-6, 1e-6, None), unknown("b")];
        assert_eq!(data(&rows, 10).unknown_rows(), vec![1]);
    }
}
