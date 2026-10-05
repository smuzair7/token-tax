use super::fmt::{self, Tier};
use super::style::ansi;
use super::table::TableData;
use crate::pricing::Row;

/// Renders the table as plain ANSI text and returns it.
///
/// No terminal state is touched, so this is safe for pipes, files, and CI logs.
/// Colour is emitted only when `color` is true, which the caller decides from
/// `stdout.is_terminal()` and `NO_COLOR`.
pub fn render(data: &TableData<'_>, color: bool) -> String {
    let mut out = String::with_capacity(2048);
    let max = data.max_total().unwrap_or(0.0);
    let cheapest = data.cheapest_index();

    push_header(&mut out, data, color);

    let (id_w, ctx_w, pin_w, pout_w, cost_w, bar_w) = column_widths(data);

    // Header
    out.push_str(&pad("MODEL", id_w, color, ansi::BOLD, ansi::BLUE));
    out.push_str(&pad("CONTEXT", ctx_w, color, ansi::BOLD, ansi::BLUE));
    out.push_str(&pad("$/MTOK IN", pin_w, color, ansi::BOLD, ansi::BLUE));
    out.push_str(&pad("$/MTOK OUT", pout_w, color, ansi::BOLD, ansi::BLUE));
    out.push_str(&pad("INPUT $", cost_w, color, ansi::BOLD, ansi::BLUE));
    out.push_str(&pad("OUTPUT $", cost_w, color, ansi::BOLD, ansi::BLUE));
    out.push_str(&pad("TOTAL $", cost_w, color, ansi::BOLD, ansi::BLUE));
    out.push_str("  ");
    out.push_str(&styled("BAR", color, ansi::BOLD, ansi::BLUE));
    out.push('\n');

    let span = id_w + ctx_w + pin_w + pout_w + cost_w * 3 + 2 + bar_w;
    out.push_str(&rule(span, color));

    for (i, row) in data.rows.iter().enumerate() {
        push_row(
            &mut out,
            row,
            data,
            Some(i) == cheapest,
            max,
            id_w,
            ctx_w,
            pin_w,
            pout_w,
            cost_w,
            bar_w,
            color,
        );
    }

    out.push_str(&rule(span, color));
    push_footer(&mut out, data, color);

    out
}

fn push_header(out: &mut String, data: &TableData<'_>, color: bool) {
    out.push_str(&styled(
        &format!("{} prompt tokens", fmt::count(data.input_tokens)),
        color,
        ansi::BOLD,
        ansi::CYAN,
    ));
    out.push_str(&styled(
        &format!(
            "  ({} bytes, {})\n",
            fmt::count(data.byte_count as u64),
            data.encoding
        ),
        color,
        ansi::DIM,
        ansi::GREY_244,
    ));

    if data.source_is_stale {
        out.push_str(&styled(
            &format!(
                "STALE PRICING: using {} - {}\n",
                data.source,
                data.fetch_error.unwrap_or("fetch failed")
            ),
            color,
            ansi::BOLD,
            ansi::YELLOW,
        ));
    }
    out.push('\n');
}

#[allow(clippy::too_many_arguments)]
fn push_row(
    out: &mut String,
    row: &Row,
    data: &TableData<'_>,
    is_cheapest: bool,
    max: f64,
    id_w: usize,
    ctx_w: usize,
    pin_w: usize,
    pout_w: usize,
    cost_w: usize,
    bar_w: usize,
    color: bool,
) {
    let Some(price) = row.price.as_ref() else {
        out.push_str(&pad(
            &fmt::ellipsize(&row.requested, id_w),
            id_w,
            color,
            ansi::DIM,
            ansi::GREY_244,
        ));
        // Fill the numeric span so the row's right edge still lines up.
        out.push_str(&pad(
            "unknown model",
            ctx_w + pin_w + pout_w + cost_w * 3,
            color,
            ansi::DIM,
            ansi::YELLOW,
        ));
        out.push_str(&" ".repeat(bar_w));
        if !row.suggestions.is_empty() {
            out.push_str(&styled(
                &format!("  {}", row.suggestions.join(", ")),
                color,
                ansi::DIM,
                ansi::GREY_244,
            ));
        }
        out.push('\n');
        return;
    };

    let input = price.input_cost(data.input_tokens);
    let output = price.output_cost(data.output_tokens);
    let total = price.total_cost(data.input_tokens, data.output_tokens);

    let mut name = fmt::ellipsize(&row.requested, id_w);
    if !price.fits(data.input_tokens, data.output_tokens) {
        name.push('!');
    }
    out.push_str(&pad(&name, id_w, color, ansi::DIM, ansi::GREY_244));

    let ctx = price
        .context_length
        .map(fmt::context)
        .unwrap_or_else(|| "n/a".to_string());
    out.push_str(&pad(&ctx, ctx_w, color, ansi::DIM, ansi::GREY_244));
    out.push_str(&pad(
        &fmt::usd_per_mtok(price.prompt_per_mtok()),
        pin_w,
        color,
        ansi::DIM,
        ansi::GREY_244,
    ));
    out.push_str(&pad(
        &fmt::usd_per_mtok(price.completion_per_mtok()),
        pout_w,
        color,
        ansi::DIM,
        ansi::GREY_244,
    ));

    out.push_str(&cost_cell(&fmt::usd(input), cost_w, Tier::of(input), color));
    out.push_str(&cost_cell(
        &fmt::usd(output),
        cost_w,
        Tier::of(output),
        color,
    ));

    if is_cheapest {
        // The marker goes after the newline so the numeric columns above stay
        // aligned with every other row.
        out.push_str(&cost_cell(&fmt::usd(total), cost_w, Tier::of(total), color));
        out.push_str(&styled("  <- cheapest", color, ansi::BOLD, ansi::GREEN));
    } else {
        out.push_str(&cost_cell(&fmt::usd(total), cost_w, Tier::of(total), color));
    }

    out.push(' ');
    out.push_str(&styled(
        &fmt::bar(total, max, bar_w),
        color,
        ansi::DIM,
        ansi::MAGENTA,
    ));
    out.push('\n');
}

fn push_footer(out: &mut String, data: &TableData<'_>, color: bool) {
    let mut note = format!(
        "prices: {}  |  assumes {} output tokens  |  {:.0}ms",
        data.source,
        fmt::count(data.output_tokens),
        data.elapsed_ms
    );
    let unknown = data.unknown_rows().len();
    if unknown > 0 {
        note.push_str(&format!("  |  {unknown} model(s) unresolved"));
    }
    out.push('\n');
    out.push_str(&styled(&note, color, ansi::DIM, ansi::GREY_244));
    out.push('\n');
}

/// Column widths, derived from the data rather than fixed, so long vendor
/// prefixes do not wrap and short ids leave no dead space.
fn column_widths(data: &TableData<'_>) -> (usize, usize, usize, usize, usize, usize) {
    let id_w = data
        .rows
        .iter()
        .map(|r| r.requested.chars().count())
        .max()
        .unwrap_or(5)
        .clamp(5, 46);
    let ctx_w = 7;
    let pin_w = 9;
    let pout_w = 10;
    let cost_w = data
        .rows
        .iter()
        .filter_map(|r| data.total_for(r))
        .map(|t| fmt::usd(t).chars().count())
        .chain([fmt::usd(0.0).chars().count()])
        .max()
        .unwrap_or(6)
        .clamp(6, 14);
    let bar_w = 12;
    (id_w, ctx_w, pin_w, pout_w, cost_w, bar_w)
}

/// Left-aligns `text` in `width`, applying colour only when `color`.
fn pad(text: &str, width: usize, color: bool, attr: &str, color_code: &str) -> String {
    let len = text.chars().count();
    let mut s = styled(text, color, attr, color_code);
    for _ in len..width {
        s.push(' ');
    }
    s.push(' ');
    s
}

/// Right-aligns a cost value in `width`, coloured by tier.
fn cost_cell(text: &str, width: usize, tier: Tier, color: bool) -> String {
    let code = match tier {
        Tier::Cheap => ansi::GREEN,
        Tier::Moderate => ansi::YELLOW,
        Tier::Expensive => ansi::RED,
    };
    let len = text.chars().count();
    let mut s = String::new();
    for _ in len..width {
        s.push(' ');
    }
    if color {
        s.push_str(code);
        s.push_str(text);
        s.push_str(ansi::RESET);
    } else {
        s.push_str(text);
    }
    s.push(' ');
    s
}

fn rule(width: usize, color: bool) -> String {
    styled(&"-".repeat(width), color, ansi::DIM, ansi::GREY_244) + "\n"
}

fn styled(text: &str, color: bool, attr: &str, color_code: &str) -> String {
    if color {
        ansi::paint(attr, color_code, text)
    } else {
        text.to_string()
    }
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
            input_tokens: 1234,
            output_tokens: 1000,
            byte_count: 5000,
            encoding: "cl100k_base",
            source: if stale { "offline snapshot" } else { "live" },
            source_is_stale: stale,
            fetch_error: if stale {
                Some("could not reach openrouter.ai")
            } else {
                None
            },
            elapsed_ms: 42.0,
        }
    }

    fn sample() -> Vec<Row> {
        vec![
            priced("openai/gpt-4o", 2.5e-6, 1e-5, Some(128_000)),
            priced("anthropic/claude-3.5-sonnet", 3e-6, 1.5e-5, Some(200_000)),
            priced("tiny/free-model", 0.0, 0.0, None),
        ]
    }

    #[test]
    fn plain_render_has_no_escape_sequences() {
        let rows = sample();
        let out = render(&data(&rows, false), false);
        assert!(!out.contains('\x1b'), "colour leaked into plain output");
        assert!(!out.contains('\r'));
    }

    #[test]
    fn color_render_includes_escapes_and_resets() {
        let rows = sample();
        let out = render(&data(&rows, true), true);
        assert!(out.contains('\x1b'));
        let (groups, resets) = count_style_groups(&out);
        assert!(groups > 5, "expected several styled cells, got {groups}");
        assert_eq!(
            groups, resets,
            "every style group must be closed by exactly one reset"
        );
    }

    /// Counts styled groups in an escape stream.
    ///
    /// A cell emits an attribute sequence *and* a colour sequence before its
    /// text, so consecutive non-reset escapes are one group, not two. Returns
    /// `(groups_opened, resets_seen)`.
    fn count_style_groups(out: &str) -> (usize, usize) {
        let mut groups = 0;
        let mut resets = 0;
        let mut open = false;

        let mut rest = out;
        while let Some(idx) = rest.find('\x1b') {
            let seq_start = idx;
            let after = &rest[idx..];
            // SGR sequences end at the first 'm'.
            let seq = match after.find('m') {
                Some(end) => &after[..=end],
                None => break,
            };
            rest = &after[seq.len()..];

            if seq == ansi::RESET {
                resets += 1;
                open = false;
            } else if !open {
                groups += 1;
                open = true;
            }
            let _ = seq_start;
        }

        assert!(!open, "output ended with a style still open");
        (groups, resets)
    }

    #[test]
    fn header_reports_token_counts_and_encoding() {
        let rows = sample();
        let out = render(&data(&rows, false), false);
        assert!(out.contains("1,234 prompt tokens"));
        assert!(out.contains("cl100k_base"));
        assert!(out.contains("1,000 output tokens"));
    }

    #[test]
    fn stale_banner_appears_only_when_stale() {
        let rows = sample();
        assert!(!render(&data(&rows, false), false).contains("STALE"));
        let stale = render(&data(&rows, true), false);
        assert!(stale.contains("STALE PRICING"));
        assert!(stale.contains("could not reach openrouter.ai"));
    }

    #[test]
    fn cheapest_row_is_marked_exactly_once() {
        let rows = sample();
        let out = render(&data(&rows, false), false);
        assert_eq!(out.matches("<- cheapest").count(), 1);
        assert!(out.contains("tiny/free-model"));
    }

    #[test]
    fn unknown_rows_show_suggestions_and_unresolved_count() {
        let rows = vec![unknown(
            "claude-3.5-sonnet",
            vec!["anthropic/claude-3.5-sonnet"],
        )];
        let out = render(&data(&rows, false), false);
        assert!(out.contains("unknown model"));
        assert!(out.contains("anthropic/claude-3.5-sonnet"));
        assert!(out.contains("1 model(s) unresolved"));
    }

    #[test]
    fn context_overflow_is_flagged_in_plain_output() {
        let rows = vec![priced("tiny/model", 1e-6, 1e-6, Some(10))];
        let out = render(&data(&rows, false), false);
        assert!(out.contains('!'), "overflow marker missing");
    }

    #[test]
    fn one_line_per_row_plus_frame() {
        let rows = sample();
        let out = render(&data(&rows, false), false);
        let model_lines = out
            .lines()
            .filter(|l| {
                l.contains("openai/gpt-4o")
                    || l.contains("anthropic/claude-3.5-sonnet")
                    || l.contains("tiny/free-model")
            })
            .count();
        assert_eq!(model_lines, 3);
    }

    #[test]
    fn empty_rows_render_without_panicking() {
        let out = render(&data(&[], true), false);
        assert!(out.contains("STALE PRICING"));
    }

    #[test]
    fn very_long_ids_are_ellipsized_not_wrapped() {
        let long = "vendor-with-an-extremely-long-name/with-a-long-model-suffix-and-more";
        let rows = vec![priced(long, 1e-6, 1e-6, Some(1000))];
        let out = render(&data(&rows, false), false);
        assert!(out.contains('…'), "expected an ellipsis in {out}");
    }
}
