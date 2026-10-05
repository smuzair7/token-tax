//! Formats numbers for the cost table.

/// Formats USD with enough precision to stay meaningful at token scale.
///
/// A prompt priced per-token lands around $0.0000015, so two decimal places
/// would round nearly every row to `$0.00` and destroy the comparison. This
/// scales precision to magnitude: sub-cent values get up to 6 significant
/// decimals, small values 4, and anything larger falls back to the conventional
/// two.
pub fn usd(value: f64) -> String {
    render_money(value, 4, 2)
}

/// Renders a money value keeping at least `sig` significant digits.
///
/// This table exists to compare costs, and token-scale prices span nine orders
/// of magnitude ($0.000000075 to $75.00). A fixed number of decimals — two for
/// currency, or even six — silently collapses distinct values into the same
/// string, so precision follows the magnitude of the number instead. `min_dec`
/// keeps familiar values like `$3.00` looking conventional.
fn render_money(value: f64, sig: u32, min_dec: u32) -> String {
    if !value.is_finite() {
        return "n/a".to_string();
    }
    if value == 0.0 {
        return format!("${value:.prec$}", prec = min_dec as usize);
    }

    let magnitude = value.abs().log10().floor() as i32;
    // Decimal places needed to express `sig` significant digits.
    let needed = sig as i32 - 1 - magnitude;
    let decimals = needed.max(min_dec as i32).clamp(0, 12) as usize;
    format!("${value:.decimals$}")
}

/// Formats a $/Mtok rate, which spans several orders of magnitude between a
/// small model and a frontier one.
pub fn usd_per_mtok(value: f64) -> String {
    // Rates run from single-digit cents (Gemini Flash) to hundreds of dollars,
    // so they get one more significant digit than totals.
    render_money(value, 3, 2)
}

/// Thousands-separated integer, e.g. `1234567` -> `1,234,567`.
pub fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Compacts a context window: `1,048,576` -> `1049k`, `200000` -> `200k`.
pub fn context(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{}M", value / 1_000_000)
    } else if value >= 1_000 {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
}

/// Cost tier, used to pick the cell colour.
///
/// Thresholds are absolute because a cost comparison is only meaningful against
/// other rows in the same table: $0.02 reads as "fine" for a 1M-token batch and
/// as "expensive" for a single short prompt. The middle tier is deliberately
/// wide so ordinary short-prompt runs stay green.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Under a cent.
    Cheap,
    /// Under ten cents.
    Moderate,
    /// Ten cents or more.
    Expensive,
}

impl Tier {
    pub fn of(total: f64) -> Self {
        if !total.is_finite() {
            return Tier::Expensive;
        }
        if total < 0.01 {
            Tier::Cheap
        } else if total < 0.10 {
            Tier::Moderate
        } else {
            Tier::Expensive
        }
    }
}

/// Horizontal bar of `width` cells, scaled against `max`.
///
/// Returns a full bar for the largest value and proportionally shorter ones
/// otherwise. A zero `max` (every model free, or all rows unknown) yields spaces
/// rather than dividing by zero.
pub fn bar(value: f64, max: f64, width: usize) -> String {
    if width == 0 || !value.is_finite() || max <= 0.0 || value <= 0.0 {
        return " ".repeat(width);
    }
    let filled = ((value / max) * width as f64)
        .round()
        .clamp(0.0, width as f64) as usize;
    // Always show at least one cell for a non-zero cost, so a cheap model is
    // still visibly cheaper than one that is genuinely free.
    let filled = if filled == 0 { 1 } else { filled };
    let mut s = String::with_capacity(width * 3);
    s.push_str(&"█".repeat(filled));
    s.push_str(&" ".repeat(width - filled));
    s
}

/// Truncates to `width` with a trailing ellipsis, for model IDs that overflow.
pub fn ellipsize(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    if width == 1 {
        return "…".to_string();
    }
    let mut s: String = chars[..width - 1].iter().collect();
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usd_keeps_sub_cent_precision() {
        // Four significant digits at every magnitude: enough to separate real
        // costs, few enough to stay readable.
        assert_eq!(usd(0.0000015), "$0.000001500");
        assert_eq!(usd(0.018), "$0.01800");
        assert_eq!(usd(0.25), "$0.2500");
        assert_eq!(usd(12.5), "$12.50");
        assert_eq!(usd(1234.5), "$1234.50");
    }

    #[test]
    fn money_precision_follows_magnitude() {
        // A sub-cent value keeps its significant digits by gaining decimals.
        assert_eq!(usd(0.0000015), "$0.000001500");
        // 7.5e-8 per token is the smallest real rate in the catalog; it needs 11
        // decimal places to show its digits at all.
        assert_eq!(usd(0.000000075), "$0.00000007500");
        // ...and a whole-dollar value never grows decimals it does not need.
        assert_eq!(usd(75.0), "$75.00");
        assert_eq!(usd(1234.5), "$1234.50");
    }

    #[test]
    fn usd_renders_zero_as_currency() {
        assert_eq!(usd(0.0), "$0.00");
    }

    #[test]
    fn usd_never_renders_two_distinct_costs_identically() {
        // The whole table is a comparison, so adjacent realistic costs must be
        // told apart at the displayed precision.
        let mut seen = std::collections::HashSet::new();
        for prompt_per_token in [1.5e-7f64, 7.5e-8, 3.0e-6, 1.5e-5, 3.0e-5] {
            for tokens in [1u64, 137, 4_096, 250_000, 2_000_000] {
                let rendered = usd(prompt_per_token * tokens as f64);
                assert!(
                    seen.insert(rendered.clone()),
                    "collision: {prompt_per_token} x {tokens} both rendered as {rendered}"
                );
            }
        }
    }

    #[test]
    fn usd_is_monotonic_across_the_range() {
        let samples = [1e-7, 5e-7, 1e-6, 1e-4, 1e-2, 0.1, 1.0, 100.0];
        for pair in samples.windows(2) {
            let (lo, hi) = (usd(pair[0]), usd(pair[1]));
            assert_ne!(lo, hi, "collision at {pair:?}: both {lo}");
        }
    }

    #[test]
    fn usd_handles_non_finite() {
        assert_eq!(usd(f64::NAN), "n/a");
        assert_eq!(usd(f64::INFINITY), "n/a");
    }

    #[test]
    fn per_mtok_scales_precision() {
        assert_eq!(usd_per_mtok(0.075), "$0.0750");
        assert_eq!(usd_per_mtok(0.15), "$0.150");
        assert_eq!(usd_per_mtok(3.0), "$3.00");
        assert_eq!(usd_per_mtok(75.0), "$75.00");
        assert_eq!(usd_per_mtok(0.0), "$0.00");
    }

    #[test]
    fn per_mtok_separates_cheap_model_tiers() {
        // The lowest published rates are a few cents per Mtok; if these collapse
        // to the same string the cheap end of the table stops being useful.
        let tiers = [0.075, 0.15, 0.25, 0.6, 0.75, 1.25, 2.5, 3.0, 5.0, 15.0];
        let mut seen = std::collections::HashSet::new();
        for t in tiers {
            let rendered = usd_per_mtok(t);
            assert!(
                seen.insert(rendered.clone()),
                "collision at {t}: {rendered}"
            );
        }
    }

    #[test]
    fn count_adds_thousands_separators() {
        assert_eq!(count(0), "0");
        assert_eq!(count(7), "7");
        assert_eq!(count(999), "999");
        assert_eq!(count(1_000), "1,000");
        assert_eq!(count(1_234_567), "1,234,567");
        assert_eq!(count(1_048_576), "1,048,576");
    }

    #[test]
    fn context_compacts() {
        assert_eq!(context(8_192), "8k");
        assert_eq!(context(200_000), "200k");
        assert_eq!(context(1_048_576), "1M");
        assert_eq!(context(512), "512");
    }

    #[test]
    fn tiers_split_at_the_documented_thresholds() {
        assert_eq!(Tier::of(0.0), Tier::Cheap);
        assert_eq!(Tier::of(0.009_999), Tier::Cheap);
        assert_eq!(Tier::of(0.01), Tier::Moderate);
        assert_eq!(Tier::of(0.099_999), Tier::Moderate);
        assert_eq!(Tier::of(0.10), Tier::Expensive);
        assert_eq!(Tier::of(5.0), Tier::Expensive);
    }

    #[test]
    fn bar_scales_and_pads_to_fixed_width() {
        assert_eq!(bar(10.0, 10.0, 5).chars().filter(|c| *c == '█').count(), 5);
        assert_eq!(bar(0.0, 10.0, 5).chars().filter(|c| *c == '█').count(), 0);
        for (v, m) in [(1.0, 10.0), (5.0, 10.0), (2.5, 10.0), (0.001, 0.002)] {
            assert_eq!(bar(v, m, 8).chars().count(), 8);
        }
    }

    #[test]
    fn bar_never_exceeds_width_or_divides_by_zero() {
        assert_eq!(bar(100.0, 10.0, 5).chars().filter(|c| *c == '█').count(), 5);
        assert_eq!(bar(1.0, 0.0, 4).chars().filter(|c| *c == '█').count(), 0);
        assert_eq!(
            bar(f64::NAN, 1.0, 4).chars().filter(|c| *c == '█').count(),
            0
        );
    }

    #[test]
    fn bar_shows_at_least_one_cell_for_nonzero_cost() {
        assert_eq!(
            bar(0.001, 100.0, 10).chars().filter(|c| *c == '█').count(),
            1
        );
    }

    #[test]
    fn ellipsize_only_shortens_when_needed() {
        assert_eq!(ellipsize("short", 10), "short");
        assert_eq!(ellipsize("exactly-10", 10), "exactly-10");
        let out = ellipsize("anthropic/claude-3.5-sonnet", 12);
        assert_eq!(out.chars().count(), 12);
        assert!(out.ends_with('…'));
        assert_eq!(ellipsize("abcdef", 0), "");
        assert_eq!(ellipsize("abcdef", 1), "…");
    }

    #[test]
    fn ellipsize_is_char_safe() {
        let out = ellipsize("日本語のテキスト", 4);
        assert_eq!(out.chars().count(), 4);
    }
}
