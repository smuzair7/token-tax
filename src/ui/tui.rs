use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Row as RRow, Table, TableState};
use ratatui::Frame;

use super::fmt::{self, Tier};
use super::style::{self, palette};
use super::table::TableData;
use crate::pricing::Row;
use crate::term::TerminalGuard;

/// Draws the full-screen table and blocks until the user dismisses it.
///
/// The guard is created *before* the terminal is touched and is still in scope
/// for every `?` after this point, so any failure below unwinds through
/// `TerminalGuard::drop` and restores the terminal.
pub fn run(data: &TableData<'_>) -> anyhow::Result<()> {
    let mut guard = TerminalGuard::enter()?;

    let backend = ratatui::backend::CrosstermBackend::new(crate::term::out());
    let mut terminal = ratatui::Terminal::new(backend)?;
    terminal.clear()?;

    let result = (|| -> anyhow::Result<()> {
        loop {
            terminal.draw(|f| draw(f, data))?;
            if wait_for_quit()? {
                break;
            }
        }
        Ok(())
    })();

    // Restore before propagating, so a rendering error cannot leave the user in
    // the alternate screen.
    guard.release();
    result
}

/// Blocks until a quit key. `true` means "exit".
fn wait_for_quit() -> anyhow::Result<bool> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

    loop {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return Ok(true),
                    KeyCode::Char('c') if ctrl => return Ok(true),
                    _ => {}
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

/// Minimum width kept for the model-name column. Checked at compile time so a
/// bare `vendor/model` id still fits rather than truncating mid-word.
const MODEL_MIN: u16 = 16;

/// Width of every fixed column after MODEL, including inter-column spacing.
/// Sized so `MODEL_MIN + FIXED_COLUMNS == 80`: an 80-column terminal shows the
/// full numeric comparison with no bar, and wider terminals donate the surplus
/// to the bar.
const FIXED_COLUMNS: u16 = 7 + 10 + 10 + 10 + 10 + 10 + 7;

/// Widest bar rendered; beyond this the surplus stays with the model column.
const BAR_MAX: u16 = 30;

const _: () = assert!(MODEL_MIN >= 12, "model column must fit a real model id");
const _: () = assert!(
    MODEL_MIN + FIXED_COLUMNS <= 80,
    "the numeric table must fit a standard 80-column terminal"
);

/// Bar width for a given terminal width. Zero when there is no surplus, in which
/// case the `BAR` header cell is dropped rather than left dangling.
fn bar_width(available: u16) -> u16 {
    available
        .saturating_sub(MODEL_MIN + FIXED_COLUMNS)
        .min(BAR_MAX)
}

fn draw(f: &mut Frame, data: &TableData<'_>) {
    let area = f.area();

    // The summary block is two lines when a stale-pricing banner is shown and
    // one otherwise; reserving it up front keeps the table from being clipped
    // at the bottom edge.
    let summary_rows: u16 = if data.source_is_stale { 3 } else { 1 };
    let [summary, table_area, footer] = Layout::vertical([
        Constraint::Length(summary_rows),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_summary(f, summary, data);
    draw_table(f, table_area, data);
    draw_footer(f, footer, data);
}

fn draw_summary(f: &mut Frame, area: Rect, data: &TableData<'_>) {
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{} prompt tokens", fmt::count(data.input_tokens)),
            style::accent().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  ({} bytes, {})",
                fmt::count(data.byte_count as u64),
                data.encoding
            ),
            style::muted(),
        ),
    ])];

    if data.source_is_stale {
        lines.push(Line::from(Span::styled(
            format!(
                " STALE PRICING - using {} ({})",
                data.source,
                data.fetch_error.unwrap_or("fetch failed")
            ),
            style::warning(),
        )));
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn draw_table(f: &mut Frame, area: Rect, data: &TableData<'_>) {
    let max = data.max_total().unwrap_or(0.0);
    let cheapest = data.cheapest_index();
    let overflowing = data.overflowing_rows();

    let bar_cells = bar_width(area.width);

    let mut header_cells = vec![
        cell("MODEL", style::header()),
        cell("CONTEXT", style::header()),
        cell("$/MTOK IN", style::header()),
        cell("$/MTOK OUT", style::header()),
        cell("INPUT $", style::header()),
        cell("OUTPUT $", style::header()),
        cell("TOTAL $", style::header()),
    ];
    if bar_cells > 0 {
        header_cells.push(cell("BAR", style::header()));
    }
    let header = RRow::new(header_cells);

    let body: Vec<RRow> = data
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            render_row(
                row,
                data,
                Some(i) == cheapest,
                max,
                overflowing.contains(&i),
                bar_cells as usize,
            )
        })
        .collect();

    let widths = [
        Constraint::Min(MODEL_MIN),
        Constraint::Length(7),  // CONTEXT
        Constraint::Length(10), // $/MTOK IN
        Constraint::Length(10), // $/MTOK OUT
        Constraint::Length(10), // INPUT $
        Constraint::Length(10), // OUTPUT $
        Constraint::Length(10), // TOTAL $
        Constraint::Length(bar_cells),
    ];

    let table = Table::new(body, widths)
        .header(header)
        .column_spacing(1)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_type(BorderType::Plain),
        );

    // TableState is passed because the widget is rendered statefully; this
    // table is read-only, so no selection is ever set.
    f.render_stateful_widget(table, area, &mut TableState::default());
}

fn render_row<'a>(
    row: &'a Row,
    data: &TableData<'a>,
    is_cheapest: bool,
    max: f64,
    overflows: bool,
    bar_w: usize,
) -> RRow<'a> {
    RRow::new(row_cells(row, data, is_cheapest, max, overflows, bar_w))
}

/// The cells of one table row. Split out from `render_row` so tests can assert on
/// the cell count, which must track the header exactly.
///
/// `bar_w` of zero omits the trailing bar cell.
fn row_cells<'a>(
    row: &'a Row,
    data: &TableData<'a>,
    is_cheapest: bool,
    max: f64,
    overflows: bool,
    bar_w: usize,
) -> Vec<Line<'a>> {
    let mut cells = Vec::with_capacity(8);

    let Some(price) = row.price.as_ref() else {
        // Unknown models merge into a single wide message so the suggestion
        // text has room; the numeric cells stay empty and the bar is omitted.
        let mut spans = vec![
            Span::styled(row.requested.clone(), style::warning()),
            Span::raw("  "),
            Span::styled("unknown model", style::warning()),
        ];
        if !row.suggestions.is_empty() {
            spans.push(Span::styled(
                format!("  did you mean: {}", row.suggestions.join(", ")),
                style::muted(),
            ));
        }
        cells.push(Line::from(spans));
        for _ in 1..7 {
            cells.push(Line::from(""));
        }
        if bar_w > 0 {
            cells.push(Line::from(""));
        }
        return cells;
    };

    let input_cost = price.input_cost(data.input_tokens);
    let output_cost = price.output_cost(data.output_tokens);
    let total = price.total_cost(data.input_tokens, data.output_tokens);

    let mut model_spans = vec![Span::raw(row.requested.clone())];
    if overflows {
        model_spans.push(Span::styled(" !ctx", style::danger()));
    }

    let ctx = price
        .context_length
        .map(fmt::context)
        .unwrap_or_else(|| "n/a".to_string());

    cells.push(Line::from(model_spans));
    cells.push(Line::from(ctx));
    cells.push(Line::from(fmt::usd_per_mtok(price.prompt_per_mtok())));
    cells.push(Line::from(fmt::usd_per_mtok(price.completion_per_mtok())));
    cells.push(Line::from(Span::styled(
        fmt::usd(input_cost),
        style::tier_style(Tier::of(input_cost)),
    )));
    cells.push(Line::from(Span::styled(
        fmt::usd(output_cost),
        style::tier_style(Tier::of(output_cost)),
    )));
    cells.push(Line::from(if is_cheapest {
        Span::styled(format!("{} <-", fmt::usd(total)), style::cheapest_style())
    } else {
        Span::styled(fmt::usd(total), style::tier_style(Tier::of(total)))
    }));

    if bar_w > 0 {
        cells.push(Line::from(Span::styled(
            fmt::bar(total, max, bar_w),
            Style::default().fg(palette::BAR),
        )));
    }

    cells
}

fn draw_footer(f: &mut Frame, area: Rect, data: &TableData<'_>) {
    if area.height == 0 {
        return;
    }
    let unknown = data.unknown_rows().len();
    let mut note = format!(
        "prices: {}  |  assumes {} output tokens  |  {:.0}ms",
        data.source,
        fmt::count(data.output_tokens),
        data.elapsed_ms
    );
    if unknown > 0 {
        note.push_str(&format!("  |  {unknown} unresolved"));
    }
    f.render_widget(Paragraph::new(note), area);
}

fn cell<'a>(text: &'a str, style: Style) -> ratatui::widgets::Cell<'a> {
    ratatui::widgets::Cell::from(Span::styled(text, style))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::ModelPrice;

    fn priced(id: &str, p: f64, c: f64, ctx: Option<u64>) -> Row {
        Row {
            requested: id.to_string(),
            price: Some(ModelPrice {
                id: id.to_string(),
                prompt_per_token: p,
                completion_per_token: c,
                context_length: ctx,
            }),
            suggestions: vec![],
        }
    }

    fn unknown(id: &str, suggestions: Vec<&str>) -> Row {
        Row {
            requested: id.to_string(),
            price: None,
            suggestions: suggestions.into_iter().map(str::to_string).collect(),
        }
    }

    fn data<'a>(rows: &'a [Row], stale: bool) -> TableData<'a> {
        TableData {
            rows,
            input_tokens: 1000,
            output_tokens: 1000,
            byte_count: 4000,
            encoding: "cl100k_base",
            source: if stale { "offline snapshot" } else { "live" },
            source_is_stale: stale,
            fetch_error: if stale { Some("offline") } else { None },
            elapsed_ms: 12.0,
        }
    }

    /// A row must always have the same number of cells as the header, or
    /// ratatui renders garbage. This is the invariant that broke when the bar
    /// was made width-dependent, so it is asserted directly.
    #[test]
    fn row_cell_count_matches_header_with_and_without_bar() {
        let rows = vec![
            priced("openai/gpt-4o", 2.5e-6, 1e-5, Some(128_000)),
            priced("free/model", 0.0, 0.0, Some(8_000)),
            priced("no/ctx", 1e-6, 1e-6, None),
            unknown("typo/model", vec!["real/model"]),
            unknown("no/hints", vec![]),
        ];
        let d = data(&rows, true);
        let max = d.max_total().unwrap();
        let cheapest = d.cheapest_index();
        let overflowing = d.overflowing_rows();

        // 7 fixed columns plus the bar when one is rendered.
        for (with_bar, expected) in [(0usize, 7usize), (10, 8), (30, 8)] {
            for (i, r) in rows.iter().enumerate() {
                let cells = row_cells(
                    r,
                    &d,
                    Some(i) == cheapest,
                    max,
                    overflowing.contains(&i),
                    with_bar,
                );
                assert_eq!(
                    cells.len(),
                    expected,
                    "bar={with_bar} row={} had wrong cell count",
                    r.requested
                );
            }
        }
    }

    #[test]
    fn render_row_never_panics_for_priced_rows() {
        let rows = vec![
            priced("openai/gpt-4o", 2.5e-6, 1e-5, Some(128_000)),
            priced("free/model", 0.0, 0.0, Some(8_000)),
            priced("no/ctx", 1e-6, 1e-6, None),
        ];
        let d = data(&rows, false);
        let max = d.max_total().unwrap();
        let cheapest = d.cheapest_index();
        for (i, r) in rows.iter().enumerate() {
            let _ = render_row(r, &d, Some(i) == cheapest, max, false, 12);
        }
    }

    #[test]
    fn render_row_handles_unknown_with_and_without_suggestions() {
        let rows = vec![
            unknown("typo/model", vec!["real/model", "other/model"]),
            unknown("no/hints", vec![]),
        ];
        let d = data(&rows, false);
        for r in &rows {
            let _ = render_row(r, &d, false, 0.0, false, 12);
            let _ = render_row(r, &d, false, 0.0, false, 0);
        }
    }

    #[test]
    fn overflow_marker_path_renders() {
        let rows = vec![priced("tight/model", 1e-6, 1e-6, Some(10))];
        let d = data(&rows, false);
        let overflowing = d.overflowing_rows();
        let _ = render_row(&rows[0], &d, false, 1.0, overflowing.contains(&0), 12);
        assert_eq!(overflowing, vec![0]);
    }

    #[test]
    fn bar_width_grows_with_terminal_then_saturates() {
        assert_eq!(bar_width(80), 0, "no surplus at 80 columns");
        assert_eq!(bar_width(100), 20);
        assert_eq!(bar_width(300), BAR_MAX, "capped");
        assert_eq!(bar_width(40), 0, "no underflow on a tiny terminal");
        assert_eq!(bar_width(0), 0);
    }

    /// Renders into a test backend and returns the visible text, so tests can
    /// assert on what a user would actually see rather than on internals.
    fn buffer_text(width: u16, height: u16, data: &TableData<'_>) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, data)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn bar_column_appears_only_when_the_terminal_has_room() {
        let rows = vec![priced("a/b", 1e-6, 2e-6, Some(1000))];
        let d = data(&rows, false);

        let narrow = buffer_text(80, 20, &d);
        assert!(narrow.contains("TOTAL $"), "header missing at 80 columns");
        assert!(
            !narrow.contains("BAR"),
            "BAR column should not be drawn without spare width"
        );

        let wide = buffer_text(140, 20, &d);
        assert!(wide.contains("BAR"), "BAR column missing at 140 columns");
    }

    #[test]
    fn numeric_columns_fit_an_eighty_column_terminal() {
        let rows = vec![priced("openai/gpt-4o", 2.5e-6, 1e-5, Some(128_000))];
        let d = data(&rows, false);
        let text = buffer_text(80, 20, &d);
        for header in ["MODEL", "CONTEXT", "INPUT $", "OUTPUT $", "TOTAL $"] {
            assert!(
                text.contains(header),
                "{header:?} did not fit at 80 columns:\n{text}"
            );
        }
    }

    #[test]
    fn draw_into_a_small_buffer_does_not_panic() {
        // The real risk in a ratatui UI is arithmetic on a cramped area
        // producing a zero or negative constraint.
        let rows = vec![priced("a/b", 1e-6, 2e-6, Some(1000))];
        let d = data(&rows, true);
        for (w, h) in [(20u16, 6u16), (40, 10), (80, 24), (200, 60)] {
            let backend = ratatui::backend::TestBackend::new(w, h);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal.draw(|f| draw(f, &d)).unwrap();
        }
    }

    #[test]
    fn draw_with_no_rows_does_not_panic() {
        let d = data(&[], true);
        let backend = ratatui::backend::TestBackend::new(60, 12);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &d)).unwrap();
    }
}
