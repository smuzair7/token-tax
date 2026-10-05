# token-tax

Cost calculator for LLM CLI agents. Reads a prompt on stdin, counts the exact
tokens, fetches live pricing from OpenRouter, and prints what that prompt would
cost across several models.

```
$ git diff | token-tax
27 prompt tokens  (126 bytes, cl100k_base)

MODEL                     CONTEXT $/MTOK IN $/MTOK OUT INPUT $    OUTPUT $   TOTAL $      BAR
---------------------------------------------------------------------------------------------
openai/gpt-4o             128k    $2.50     $10.00     $0.0000675 $0.01000   $0.01007  █████████████
anthropic/claude-sonnet-4 200k    $3.00     $15.00     $0.0000810 $0.01500   $0.01508  ████████████████████
google/gemini-2.5-flash   1M      $0.300    $2.50      $0.0000081 $0.002500  $0.002508  ███

prices: live  |  assumes 1,000 output tokens  |  118ms
```

## Install

```sh
cargo install --git https://github.com/smuzair7/token-tax
```

## Usage

```sh
git diff | token-tax                              # compare the default models
cat large.rs | token-tax -m openai/gpt-4o -o 4000 # one model, 4000 output tokens
pbpaste | token-tax -e o200k_base                 # use the GPT-4o tokenizer
prompt.txt | token-tax > report.txt               # plain output when redirected
```

| Flag | Default | Description |
| --- | --- | --- |
| `-m`, `--models` | `openai/gpt-4o-mini,anthropic/claude-sonnet-4,google/gemini-2.5-flash` | Comma-separated OpenRouter model IDs |
| `-o`, `--output-tokens` | `1000` | Estimated completion tokens to price in |
| `-e`, `--encoding` | `cl100k_base` | Tokenizer: `cl100k_base`, `p50k_base`, `r50k_base`, `o200k_base` |

Input tokens are counted from stdin; output tokens are the `-o` estimate, since
the real count is not knowable before the call.

## Output

| Column | Meaning |
| --- | --- |
| `CONTEXT` | Total context window; `!` on the model name if input + output exceeds it |
| `$/MTOK IN` / `$/MTOK OUT` | Price per million input and output tokens |
| `INPUT $` / `OUTPUT $` / `TOTAL $` | Cost at the assumed token counts |
| `BAR` | Total cost relative to the most expensive row; shown when the terminal has spare width |

Cost cells are coloured by magnitude: green under $0.01, yellow under $0.10, red
above. The cheapest row is marked. The `BAR` header is dropped rather than left
dangling when there is no room for it.

## Behaviour

- **Offline fallback.** If OpenRouter is unreachable the tool uses a compiled-in
  price snapshot and prints a `STALE PRICING` banner with the reason, so you can
  tell whether to trust the numbers. The fetch times out after 2.5s.
- **Unknown model IDs** render as an `unknown model` row with nearest matches
  suggested, on stderr and in the table. The exit code stays `0`.
- **Display adapts to the terminal.** Full-screen table on a capable TTY, held
  until `q`, `Esc`, or `Ctrl+C`. Plain ANSI table when stdout is redirected, when
  `TERM` is `dumb` or unset, or when another program already owns raw mode.
- **Respects `NO_COLOR`.**

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Table rendered |
| `1` | Failure |
| `2` | Nothing on stdin |

## Development

```sh
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Tests need no network access. MSRV is 1.88, set by the dependency tree.

| Module | Responsibility |
| --- | --- |
| `cli` | Argument parsing and defaults |
| `input` | stdin reading and size guard |
| `tokens` | Encoding selection and token counting |
| `pricing` | OpenRouter fetch, offline snapshot, model resolution |
| `term` | Terminal setup and restoration |
| `ui` | Table data plus the TUI and plain renderers |

## License

MIT
