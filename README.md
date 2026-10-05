# token-tax

A pre-flight cost calculator for LLM CLI agents. Pipe a prompt in, and
`token-tax` counts the exact tokens, fetches live pricing from OpenRouter, and
shows what that prompt would cost across several models before you send it.

```
$ git diff | token-tax
27 prompt tokens  (126 bytes, cl100k_base)

MODEL                     CONTEXT $/MTOK IN $/MTOK OUT INPUT $    OUTPUT $   TOTAL $      BAR
------------------------------------------------------------------------------------------------
openai/gpt-4o             128k    $2.50     $10.00     $0.0000675 $0.01000   $0.01007  █████████████
anthropic/claude-sonnet-4 200k    $3.00     $15.00     $0.0000810 $0.01500   $0.01508  ████████████████████
google/gemini-2.5-flash   1M      $0.300    $2.50      $0.0000081 $0.002500  $0.002508  ███

prices: live  |  assumes 1,000 output tokens  |  118ms
```

## Install

```sh
cargo install --path .
```

## Usage

```sh
git diff | token-tax
cat large.rs | token-tax -m openai/gpt-4o -o 4000
pbpaste | token-tax -e o200k_base
prompt.txt | token-tax > report.txt
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `-m`, `--models` | `openai/gpt-4o-mini,anthropic/claude-sonnet-4,google/gemini-2.5-flash` | Comma-separated OpenRouter model IDs |
| `-o`, `--output-tokens` | `1000` | Estimated completion tokens to price in |
| `-e`, `--encoding` | `cl100k_base` | Tokenizer: `cl100k_base`, `p50k_base`, `r50k_base`, `o200k_base` |

Exit codes: `0` table rendered, `1` failure, `2` nothing piped on stdin.

## Behaviour worth knowing

**It degrades, it does not fail.** If OpenRouter is unreachable, the tool falls
back to a compiled-in price snapshot for ~18 models and prints an amber
`STALE PRICING` banner with the reason. You always get a table; the banner tells
you whether to trust it. The fetch times out at 2.5s.

**Unknown model IDs do not fail the run.** A misspelled ID renders as an
`unknown model` row with the nearest catalog IDs suggested, on stderr and in the
table. Exiting non-zero for a typo would be hostile in a shell loop.

Suggestions combine substring containment with Jaro-Winkler, because neither
works alone on OpenRouter IDs. The dominant error is dropping the `vendor/`
prefix, and Jaro-Winkler is the wrong tool for it — it divides by string length,
so `gpt-4o` scored 0.33 against its own match `openai/gpt-4o` while an unrelated
long ID sharing three scattered characters scored 0.74. Containment is handled
directly, and the typo path is gated on length ratio. Queries shorter than three
characters get no suggestions at all; they are contained in too much of the
catalog to carry intent.

**Two renderers, chosen automatically.** On a capable TTY you get a full-screen
`ratatui` table held until `q`, `Esc`, or `Ctrl+C`. When stdout is redirected,
or `TERM` is `dumb`/unset, or another program already owns raw mode, it prints a
plain ANSI table and exits instead — so `token-tax > report.txt`, `| less`, and
CI logs all behave. Colour is suppressed when `NO_COLOR` is set.

**The bar column is width-dependent.** Fixed columns plus the model name fit an
80-column terminal exactly; the bar appears only when there is surplus width, and
its header is dropped rather than left dangling.

**Cost precision follows magnitude.** Token-scale prices span nine orders of
magnitude, so a fixed two decimals would render every row as `$0.00`. Values are
formatted to four significant digits instead, which keeps `$0.00000007500` and
`$12.50` distinguishable.

**Context windows are checked.** A row whose input plus output exceeds the
model's window is marked `!`. A model with no published context length is never
flagged.

## Terminal restoration

`TerminalGuard` owns raw mode, the alternate screen, and cursor visibility, and
restores all three from `Drop` — each independently, so one failure cannot skip
the others. Success paths call `release()` to disarm it; panics and early
returns rely on unwinding through `Drop`. `Ctrl+C` is delivered as a key event
rather than a signal, so it flows through the same exit path.

`SIGTERM`, `SIGHUP`, and `SIGINT` are handled by a detached thread that restores
the terminal before exiting 128. Without it, `kill <pid>` or a closing SSH
window leaves an unusable shell. `SIGKILL` is the one exception — it cannot be
caught by any process, so it can always strand a terminal.

If something else already owns raw mode (an outer TUI, some SSH wrappers), the
tool detects it and declines to take the alternate screen rather than tearing
down a session it does not own.

## Performance

Startup is single-digit milliseconds for a typical prompt: the BPE table is built
lazily on first use, the tokio runtime is `current_thread`, and the network wait
happens after tokenization so it never counts toward local work. Measured on a
98 KB input: ~36ms total, of which ~8ms is tokenizing 26,247 tokens. A live
pricing fetch adds ~80ms.

## Design notes

Module layout:

| Module | Responsibility |
| --- | --- |
| `cli` | Argument parsing, defaults, model-list splitting |
| `input` | stdin terminal detection, size guard |
| `tokens` | Encoding selection and exact token counting |
| `pricing` | OpenRouter fetch, offline snapshot, model resolution, Jaro-Winkler fuzzy match |
| `term` | Terminal setup with guaranteed restoration |
| `ui` | Shared `TableData`, plus the TUI and plain renderers |

Two deliberate choices: OpenRouter returns prices as *decimal strings* in USD per
single token (`"0.000003"` = $3/Mtok), and both the live path and the compiled-in
snapshot store that same per-token value, so the two can never disagree about
scale. Models missing either half of their price are dropped from the catalog
rather than defaulted — half a price cannot be filled in without inventing a
number, so they surface as `UNKNOWN` instead of a wrong cost.

The snapshot prices drift from live list prices over time. That is the accepted
trade-off for having a working tool offline; the banner is what keeps it honest.

## Development

```sh
cargo test                      # unit tests, no network required
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build --release
cargo install --path .          # or: cargo install --git <url>
```

Tests cover price parsing (malformed, null, bare-number, sentinel values), cost
arithmetic, encoding resolution, fuzzy-match ranking, CLI defaults, number
formatting collisions, terminal capability probing, and both renderers at
terminal sizes from 20 to 300 columns.

MSRV is **1.88**, set by the dependency tree (`darling` via ratatui, `icu_*` via
reqwest) rather than by this crate's own code. `cargo +1.88.0 build` verifies it.

## License

MIT
