use ratatui::style::{Color, Modifier, Style};

use super::fmt::Tier;

/// Palette shared by both renderers.
///
/// Colours are chosen to stay legible on both light and dark backgrounds: the
/// bright variants for cyan/magenta rather than the dim ones, and cost tiers
/// that rely on hue plus a textual marker rather than hue alone.
pub mod palette {
    use ratatui::style::Color;

    pub const ACCENT: Color = Color::Cyan;
    pub const MUTED: Color = Color::DarkGray;
    pub const HEADER: Color = Color::Blue;
    pub const WARNING: Color = Color::Yellow;
    pub const DANGER: Color = Color::Red;
    pub const OK: Color = Color::Green;
    pub const BAR: Color = Color::Magenta;
    pub const CHEAPEST: Color = Color::Green;
}

/// Maps a cost tier to its colour, for both ratatui and ANSI use.
pub fn tier_color(tier: Tier) -> Color {
    match tier {
        Tier::Cheap => palette::OK,
        Tier::Moderate => palette::WARNING,
        Tier::Expensive => palette::DANGER,
    }
}

/// ratatui style for a cost cell.
pub fn tier_style(tier: Tier) -> Style {
    Style::default().fg(tier_color(tier))
}

/// Style marking the cheapest row's total.
pub fn cheapest_style() -> Style {
    Style::default()
        .fg(palette::CHEAPEST)
        .add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::default().fg(palette::MUTED)
}

pub fn accent() -> Style {
    Style::default().fg(palette::ACCENT)
}

pub fn warning() -> Style {
    Style::default().fg(palette::WARNING)
}

pub fn danger() -> Style {
    Style::default().fg(palette::DANGER)
}

pub fn header() -> Style {
    Style::default()
        .fg(palette::HEADER)
        .add_modifier(Modifier::BOLD)
}

/// ANSI escapes, written by hand so the plain renderer has no dependency on
/// ratatui's backend machinery.
pub mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";

    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const RED: &str = "\x1b[31m";
    pub const CYAN: &str = "\x1b[36m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";

    /// 256-colour dark grey, matching ratatui's `DarkGray` closely enough that
    /// the two renderers look like the same tool.
    pub const GREY_244: &str = "\x1b[38;5;244m";

    /// Wraps `text` in an attribute plus foreground colour, always resetting
    /// afterwards so no style leaks into the next cell.
    pub fn paint(attr: &str, color: &str, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        format!("{attr}{color}{text}{RESET}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_get_distinct_colors() {
        assert_ne!(tier_color(Tier::Cheap), tier_color(Tier::Moderate));
        assert_ne!(tier_color(Tier::Moderate), tier_color(Tier::Expensive));
        assert_ne!(tier_color(Tier::Cheap), tier_color(Tier::Expensive));
    }

    #[test]
    fn cheapest_is_bold_and_green() {
        let s = cheapest_style();
        assert!(s.fg.is_some_and(|c| c == palette::CHEAPEST));
    }

    #[test]
    fn paint_always_resets_and_passes_empty_through() {
        for out in [
            ansi::paint(ansi::BOLD, ansi::GREEN, "$0.01"),
            ansi::paint(ansi::DIM, ansi::GREY_244, "1,000"),
        ] {
            assert!(out.ends_with(ansi::RESET), "missing reset in {out:?}");
            assert!(out.starts_with(ansi::BOLD) || out.starts_with(ansi::DIM));
        }
        // An empty cell must not emit bare escapes, which would show up as
        // stray colour in the middle of a row.
        assert_eq!(ansi::paint(ansi::BOLD, ansi::GREEN, ""), "");
    }

    #[test]
    fn ansi_sequences_start_with_escape() {
        assert!(ansi::GREEN.starts_with('\x1b'));
        assert!(ansi::BOLD.starts_with('\x1b'));
        assert!(ansi::GREY_244.starts_with('\x1b'));
    }
}
