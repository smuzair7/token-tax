use std::io::{self, IsTerminal, Read};

use anyhow::{bail, Context, Result};

/// Reads the whole of stdin.
///
/// Returns `None` when there is nothing meaningful to price: either stdin is an
/// interactive terminal (the user ran the command bare, so there is no piped
/// prompt) or the stream held only whitespace. Both cases exit before any
/// terminal setup happens, so a bare invocation never flashes a TUI.
pub fn read_stdin() -> Result<Option<String>> {
    let stdin = io::stdin();

    if stdin.is_terminal() {
        return Ok(None);
    }

    let mut buf = String::new();
    stdin
        .lock()
        .read_to_string(&mut buf)
        .context("failed to read prompt from stdin (is it binary or non-UTF-8?)")?;

    if buf.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(buf))
}

/// Shown when there is no piped input. Goes to stderr so that a bare run never
/// contaminates a redirected stdout.
pub fn print_no_input_hint() {
    eprintln!("token-tax: no prompt on stdin.");
    eprintln!();
    eprintln!("Pipe text in to price it, for example:");
    eprintln!("  git diff | token-tax");
    eprintln!("  cat main.rs | token-tax -m openai/gpt-4o -o 4000");
    eprintln!();
    eprintln!("For the full flag list, run: token-tax --help");
}

/// Guards against absurdly large inputs. tiktoken handles big inputs fine, but
/// a multi-gigabyte paste should not be silently turned into a multi-minute
/// tokenization with no warning.
const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;

pub fn check_size(text: &str) -> Result<()> {
    if text.len() > MAX_INPUT_BYTES {
        bail!(
            "input is {} MB, which exceeds the {} MB limit",
            text.len() / (1024 * 1024),
            MAX_INPUT_BYTES / (1024 * 1024)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_only_is_rejected() {
        assert!(check_size("   \n\t  ").is_ok());
    }

    #[test]
    fn size_limit_accepts_normal_input() {
        assert!(check_size(&"a".repeat(1024)).is_ok());
    }

    #[test]
    fn size_limit_rejects_huge_input() {
        // Build the oversized buffer without allocating 64MB twice over.
        let big = "a".repeat(MAX_INPUT_BYTES + 1);
        let err = check_size(&big).unwrap_err().to_string();
        assert!(err.contains("exceeds"), "unexpected message: {err}");
    }

    #[test]
    fn hint_goes_to_stderr() {
        // Smoke test only: confirms the function does not panic or block.
        print_no_input_hint();
    }
}
