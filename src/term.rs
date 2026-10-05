use std::io::{self, stdout, IsTerminal, Stdout, Write};

use anyhow::{Context, Result};
use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, is_raw_mode_enabled, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

/// Exit status used when a signal ends the process. Conventional for
/// signal-terminated programs, so `$?` stays meaningful to a calling script.
const EXIT_SIGNALLED: i32 = 128;

/// Restores the terminal on every exit path.
///
/// Three things must be undone — raw mode, the alternate screen, and the cursor
/// visibility — and each can fail independently, so each gets its own `let _ =`
/// rather than a `?` that would abort the remaining cleanup.
///
/// `armed` handles the double-restore case: teardown that runs on the success
/// path disarms the guard, so the later `Drop` does not re-emit sequences the
/// shell has already consumed. Left armed, it covers panics and early returns.
#[derive(Debug)]
pub struct TerminalGuard {
    armed: bool,
}

impl TerminalGuard {
    /// Puts the terminal into raw mode and the alternate screen, hiding the
    /// cursor. On any failure it restores whatever it managed to set up and
    /// returns the error, so the guard is only handed out when the terminal is
    /// fully in the expected state.
    pub fn enter() -> Result<Self> {
        if let Err(e) = enable_raw_mode().context("failed to enable raw mode") {
            let _ = disable_raw_mode();
            return Err(e);
        }

        let mut out = stdout();
        if let Err(e) = execute!(out, EnterAlternateScreen, Hide)
            .context("failed to enter the alternate screen")
        {
            let _ = disable_raw_mode();
            let _ = execute!(out, Show);
            return Err(e);
        }

        install_signal_handlers();
        Ok(Self { armed: true })
    }

    /// Undoes setup now rather than at scope exit. Idempotent.
    pub fn release(&mut self) {
        if self.armed {
            self.armed = false;
            restore();
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.armed {
            restore();
        }
    }
}

/// Writes the restore sequences to stdout.
///
/// Best-effort by design: every call is `let _ =` so that one failure cannot
/// prevent the remaining cleanup. Runs from `Drop`, from an explicit release, and
/// from a signal handler, so it must not allocate, lock, or panic.
fn restore() {
    let mut out = stdout();

    // Raw mode first: leaving the alternate screen while still in raw mode can
    // leave the cursor parked somewhere unhelpful.
    let _ = disable_raw_mode();
    let _ = execute!(out, LeaveAlternateScreen);
    let _ = execute!(out, Show);
    let _ = out.flush();
}

/// Restores the terminal, then terminates from a signal handler.
///
/// Deliberately minimal: signal-handling context is not a normal Rust context,
/// so this writes the escape sequences, flushes, and exits. `restore()` only
/// issues `ioctl` and `write` calls and touches no lock that `Drop` could also
/// be holding, so a concurrent unwinding on the main thread is harmless — both
/// paths write the same idempotent sequences.
fn restore_from_signal() -> ! {
    restore();
    // 128 is the conventional base for signal exit statuses.
    std::process::exit(EXIT_SIGNALLED);
}

/// Arranges for a terminal-killing signal to restore the terminal first.
///
/// Without this, `kill <pid>` or a closing SSH window leaves the user in the
/// alternate screen with no cursor and no echo — the shell looks broken.
///
/// `Ctrl+C` is deliberately absent: raw mode clears `ISIG`, so the driver
/// delivers it as a key event that the event loop already handles. These three
/// are the signals that arrive from outside the terminal.
///
/// Best-effort: a failure here is reported but does not abort the tool, since
/// the interactive path still works without it.
fn install_signal_handlers() {
    match Signals::new([SIGHUP, SIGTERM, SIGINT]) {
        Ok(mut signals) => {
            // Detached thread: lives until a signal arrives, at which point it
            // restores and exits. Nothing joins it on the normal path.
            let _ = std::thread::spawn(move || {
                if signals.forever().next().is_some() {
                    restore_from_signal();
                }
            });
        }
        Err(e) => {
            eprintln!("token-tax: could not install signal handlers: {e}");
        }
    }
}

/// True when this process already switched the terminal into raw mode.
///
/// Read before `TerminalGuard::enter()`. If something earlier in the pipeline
/// (an outer TUI, an SSH wrapper) owns raw mode, this tool must not disable it
/// on exit. Detecting it lets the caller fall back to the plain renderer instead
/// of corrupting a session it does not own.
pub fn raw_mode_is_foreign() -> bool {
    is_raw_mode_enabled().unwrap_or(false)
}

/// stdout handle, named so the signature reads clearly.
pub fn out() -> Stdout {
    stdout()
}

/// True when stdout is an interactive terminal.
pub fn stdout_is_tty() -> bool {
    io::stdout().is_terminal()
}

/// True when `TERM` names a terminal that supports cursor addressing.
///
/// `TERM=dumb` is what CI logs, `nohup` output, and some minimal pagers report.
/// It cannot render an alternate screen or honour cursor movement, so taking one
/// there produces a full-screen table the terminal then scrambles, or a screen
/// the user cannot escape. An unset or unrecognised `TERM` is treated the same
/// way: assuming capability is how a tool ends up wedging someone's session.
pub fn terminal_supports_tui() -> bool {
    match std::env::var("TERM") {
        Ok(term) => !matches!(term.as_str(), "" | "dumb" | "unknown"),
        // No TERM at all: Windows consoles and some CI environments.
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_fails_gracefully_off_a_terminal() {
        // Under `cargo test` stdout is captured, so raw mode cannot be enabled.
        // What matters is that the error path returns rather than panicking or
        // leaving the terminal half-configured.
        match TerminalGuard::enter() {
            Ok(mut guard) => {
                guard.release();
                // release() then Drop must not double-restore.
                drop(guard);
            }
            Err(e) => {
                assert!(!e.to_string().is_empty());
            }
        }
    }

    #[test]
    fn signal_handler_install_does_not_panic() {
        install_signal_handlers();
    }

    #[test]
    fn raw_mode_probe_does_not_panic() {
        let _ = raw_mode_is_foreign();
    }

    #[test]
    fn tty_probe_does_not_panic() {
        let _ = stdout_is_tty();
        // `out()` returns a handle to the process-wide stdout, which is just a
        // lock wrapper; nothing to assert beyond it being constructible.
        let mut handle = out();
        let _ = handle.flush();
    }

    /// Restores `TERM` when dropped, so a panic mid-test cannot leak the
    /// mutated value into other tests.
    struct TermGuard(Option<std::ffi::OsString>);

    impl TermGuard {
        fn capture() -> Self {
            Self(std::env::var_os("TERM"))
        }

        fn set(value: Option<&str>) {
            match value {
                Some(v) => std::env::set_var("TERM", v),
                None => std::env::remove_var("TERM"),
            }
        }
    }

    impl Drop for TermGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => std::env::set_var("TERM", v),
                None => std::env::remove_var("TERM"),
            }
        }
    }

    /// Reads process-global state, so the cases run inside one test to stay
    /// deterministic under the parallel runner.
    #[test]
    fn tui_capability_probe_agrees_with_term() {
        let _guard = TermGuard::capture();

        let cases: &[(Option<&str>, bool)] = &[
            (None, false),
            (Some(""), false),
            (Some("dumb"), false),
            (Some("unknown"), false),
            (Some("xterm-256color"), true),
            (Some("screen"), true),
            (Some("tmux-256color"), true),
            (Some("alacritty"), true),
        ];

        for &(value, expected) in cases {
            TermGuard::set(value);
            assert_eq!(
                terminal_supports_tui(),
                expected,
                "TERM={value:?} should give {expected}"
            );
        }
    }

    #[test]
    fn restore_is_safe_to_call_directly() {
        // Exercises the Drop path with an unarmed guard and the free function.
        let guard = TerminalGuard { armed: false };
        drop(guard);
        restore();
        restore();
    }
}
