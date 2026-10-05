mod fmt;
mod plain;
mod style;
mod table;
mod tui;

pub use table::TableData;

/// Renders the cost table and returns.
///
/// Chooses the full-screen TUI when the caller has confirmed it owns the
/// terminal; otherwise returns plain text for stdout. This is the only entry
/// point the rest of the program needs.
pub fn render(data: &TableData<'_>, interactive: bool) -> anyhow::Result<()> {
    if interactive {
        tui::run(data)
    } else {
        let color = crate::term::stdout_is_tty() && !no_color_requested();
        let text = plain::render(data, color);
        use std::io::Write;
        let mut out = crate::term::out();
        out.write_all(text.as_bytes())?;
        out.flush()?;
        Ok(())
    }
}

/// Honours the `NO_COLOR` convention (https://no-color.org).
fn no_color_requested() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::fmt::Tier;
    use super::*;
    use crate::pricing::ModelPrice;

    fn rows() -> Vec<crate::pricing::Row> {
        vec![crate::pricing::Row {
            requested: "a/b".into(),
            price: Some(ModelPrice {
                id: "a/b".into(),
                prompt_per_token: 1e-6,
                completion_per_token: 2e-6,
                context_length: Some(1000),
            }),
            suggestions: vec![],
        }]
    }

    #[test]
    fn plain_render_writes_to_stdout_without_touching_terminal_state() {
        let rows = rows();
        let data = TableData {
            rows: &rows,
            input_tokens: 10,
            output_tokens: 10,
            byte_count: 40,
            encoding: "cl100k_base",
            source: "live",
            source_is_stale: false,
            fetch_error: None,
            elapsed_ms: 1.0,
        };
        // Must not panic even though stdout is captured during tests.
        render(&data, false).unwrap();
    }

    #[test]
    fn tier_is_re_exported_for_callers() {
        assert_eq!(Tier::of(0.001), Tier::Cheap);
    }
}
