//! `token-tax` — a pre-flight cost calculator for LLM CLI agents.
//!
//! Reads a prompt on stdin, counts the exact tokens with tiktoken, prices it
//! against live OpenRouter rates, and prints a comparison table.
//!
//! Module layout:
//! - [`cli`]      — argument parsing and defaults
//! - [`input`]    — stdin detection and reading
//! - [`tokens`]   — BPE tokenizer selection and counting
//! - [`pricing`]  — OpenRouter fetch, offline snapshot, model resolution
//! - [`term`]     — terminal setup with guaranteed restoration
//! - [`ui`]       — shared table data plus the TUI and plain renderers

mod cli;
mod input;
mod pricing;
mod term;
mod tokens;
mod ui;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;

use crate::cli::Args;
use crate::pricing::Source;

/// Exit codes. Distinct values so a script can branch on *why* the run ended.
mod exit {
    /// Table rendered successfully.
    pub const OK: u8 = 0;
    /// Something failed: bad encoding, network plus no fallback, etc.
    pub const ERROR: u8 = 1;
    /// Nothing was piped in. Not an error condition for a shell pipeline.
    pub const NO_INPUT: u8 = 2;
}

/// How long to wait on OpenRouter before giving up and using the snapshot.
///
/// Kept short: the tool is meant to be snappy in a shell loop, and a stale
/// table with a banner beats blocking the pipeline on a slow API.
const PRICING_TIMEOUT: Duration = Duration::from_millis(2500);

fn main() -> ExitCode {
    // The async work is confined to the pricing fetch, so a single-threaded
    // runtime is built for it rather than paying for a multi-threaded pool on
    // every invocation. Constructed here, after argument parsing, so an early
    // exit (no stdin) never spins up the runtime at all.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("token-tax: could not start async runtime: {e}");
            return ExitCode::from(exit::ERROR);
        }
    };

    match runtime.block_on(run()) {
        Ok(code) => code,
        Err(e) => {
            // anyhow's alternate form prints the full cause chain, which matters
            // for the multi-step tokenize/fetch/resolve pipeline.
            eprintln!("token-tax: {e:#}");
            ExitCode::from(exit::ERROR)
        }
    }
}

async fn run() -> Result<ExitCode> {
    let started = Instant::now();
    let args = Args::parse();

    let Some(text) = input::read_stdin()? else {
        input::print_no_input_hint();
        return Ok(ExitCode::from(exit::NO_INPUT));
    };
    input::check_size(&text)?;

    let input_tokens = tokens::count_tokens(&text, args.encoding)?;

    let models = args.model_list();
    if models.is_empty() {
        anyhow::bail!("no models to price; pass -m with at least one OpenRouter model id");
    }

    let (catalog, source, fetch_error) = load_pricing().await;

    let rows = pricing::resolve(&models, &catalog);
    let byte_count = text.len();

    let data = ui::TableData {
        rows: &rows,
        input_tokens: input_tokens as u64,
        output_tokens: u64::from(args.output_tokens),
        byte_count,
        encoding: args.encoding.as_str(),
        source: source.label(),
        source_is_stale: source == Source::Snapshot,
        fetch_error: fetch_error.as_deref(),
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    };

    report_warnings(&data);

    // Interactive mode requires a terminal we can own: one that is a TTY, is
    // not `dumb`, and is not already in raw mode from a session we do not own.
    // Failing any of these falls back to the plain renderer rather than wedging
    // the terminal.
    let interactive =
        term::stdout_is_tty() && term::terminal_supports_tui() && !term::raw_mode_is_foreign();

    ui::render(&data, interactive)?;

    Ok(ExitCode::from(exit::OK))
}

/// Fetches live pricing, degrading to the compiled-in snapshot.
///
/// Returns `(catalog, source, error_message)`. The tool always gets a usable
/// catalog: an outage degrades the data quality, it does not fail the command.
async fn load_pricing() -> (Vec<pricing::ModelPrice>, Source, Option<String>) {
    match pricing::fetch_or_snapshot(PRICING_TIMEOUT).await {
        Ok((prices, source)) => (prices, source, None),
        Err(reason) => (pricing::snapshot(), Source::Snapshot, Some(reason)),
    }
}

/// Surfaces unresolved models on stderr, where they will not corrupt piped table
/// output. Iterates the rows directly rather than the index list, so a priced
/// row sitting between two unknown ones is not skipped.
fn report_warnings(data: &ui::TableData<'_>) {
    for row in data.rows {
        if !row.is_unknown() {
            continue;
        }
        eprintln!("token-tax: unknown model {:?}", row.requested);
        if !row.suggestions.is_empty() {
            eprintln!("  did you mean: {}", row.suggestions.join(", "));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_distinct() {
        let codes = [exit::OK, exit::ERROR, exit::NO_INPUT];
        for (i, a) in codes.iter().enumerate() {
            for b in &codes[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn verify_cli_definition() {
        use clap::CommandFactory;
        Args::command().debug_assert();
    }

    fn data<'a>(rows: &'a [pricing::Row]) -> ui::TableData<'a> {
        ui::TableData {
            rows,
            input_tokens: 10,
            output_tokens: 10,
            byte_count: 40,
            encoding: "cl100k_base",
            source: "live",
            source_is_stale: false,
            fetch_error: None,
            elapsed_ms: 1.0,
        }
    }

    fn priced(id: &str) -> pricing::Row {
        pricing::Row {
            requested: id.to_string(),
            price: Some(pricing::ModelPrice {
                id: id.to_string(),
                prompt_per_token: 1e-6,
                completion_per_token: 2e-6,
                context_length: None,
            }),
            suggestions: vec![],
        }
    }

    fn unknown(id: &str, suggestions: &[&str]) -> pricing::Row {
        pricing::Row {
            requested: id.to_string(),
            price: None,
            suggestions: suggestions.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// Regression: the original implementation sliced `rows[unknown[0]..]`,
    /// which worked only because unresolved rows happened to sort together.
    /// This asserts a priced row between two unknown ones is skipped while both
    /// unknown ones are still reported.
    #[test]
    fn warnings_report_every_unknown_row_in_any_position() {
        let rows = vec![
            unknown("a/bad-one", &["real/one"]),
            priced("vendor/good"),
            unknown("c/bad-two", &["real/two"]),
        ];
        assert_eq!(data(&rows).unknown_rows(), vec![0, 2]);

        // Report count is the assertion that matters: two warnings, not one.
        report_warnings(&data(&rows));
    }

    #[test]
    fn warnings_survive_a_mixed_row_ordering() {
        let rows = vec![
            unknown("first/bad", &[]),
            unknown("second/bad", &[]),
            priced("third/good"),
            unknown("fourth/bad", &[]),
        ];
        assert_eq!(data(&rows).unknown_rows(), vec![0, 1, 3]);
        report_warnings(&data(&rows));
    }
}
