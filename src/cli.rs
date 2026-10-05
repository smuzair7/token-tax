use std::fmt;
use std::str::FromStr;

use clap::{Parser, ValueEnum};

/// Popular, meaningfully different pricing tiers: a cheap small model, a
/// mid-tier workhorse, and a frontier model. A default that mixes tiers makes
/// the cost spread visible on a first run, which is the whole point of the tool.
///
/// All three are in the compiled-in snapshot too, so the default set still
/// resolves when the network is unavailable.
pub const DEFAULT_MODELS: &str =
    "openai/gpt-4o-mini,anthropic/claude-sonnet-4,google/gemini-2.5-flash";

pub const DEFAULT_OUTPUT_TOKENS: u32 = 1000;

#[derive(Parser, Debug)]
#[command(
    name = "token-tax",
    version,
    about = "Pre-flight cost calculator for LLM CLI agents",
    long_about = "Reads a prompt on stdin, counts the exact tokens, fetches live pricing from \
OpenRouter, and prints a cost comparison table.\n\nPipe any text in; the tool exits with \
status 2 and a usage hint if stdin is a terminal.",
    after_help = "EXAMPLES:\n  \
git diff | token-tax\n  \
cat large.rs | token-tax -m openai/gpt-4o -o 4000\n  \
pbpaste | token-tax -e o200k_base\n  \
prompt.txt | token-tax > report.txt"
)]
pub struct Args {
    /// Comma-separated OpenRouter model IDs to compare
    #[arg(short, long, value_name = "IDS", default_value = DEFAULT_MODELS)]
    pub models: String,

    /// Estimated number of output (completion) tokens to price in
    #[arg(short, long, value_name = "N", default_value_t = DEFAULT_OUTPUT_TOKENS)]
    pub output_tokens: u32,

    /// Tiktoken encoding used to count the prompt tokens
    #[arg(short, long, value_enum, default_value_t = Encoding::Cl100kBase)]
    pub encoding: Encoding,
}

/// The exact names OpenAI uses. `ValueEnum` would otherwise kebab-case these to
/// `o200k-base`, which does not match the encoding names users read in tiktoken
/// docs, so every variant names itself.
///
/// The shared `Base` postfix is kept despite clippy's `enum_variant_names`,
/// because it mirrors the upstream tiktoken spelling and keeps the enum
/// readable against that documentation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
#[allow(clippy::enum_variant_names)]
pub enum Encoding {
    /// GPT-4, GPT-3.5-turbo, Claude 3, Llama 3. The default; broadly accurate.
    #[value(name = "cl100k_base")]
    Cl100kBase,
    /// Older GPT-3 models and text-embedding-ada-002
    #[value(name = "p50k_base")]
    P50kBase,
    /// GPT-2 and Codex models
    #[value(name = "r50k_base")]
    R50kBase,
    /// GPT-4o and o-series models
    #[value(name = "o200k_base")]
    O200kBase,
}

impl Encoding {
    /// Every variant, so tests can assert the set stays in sync with `FromStr`
    /// and with clap's accepted spellings.
    #[cfg(test)]
    pub const ALL: [Encoding; 4] = [
        Encoding::Cl100kBase,
        Encoding::P50kBase,
        Encoding::R50kBase,
        Encoding::O200kBase,
    ];
}

impl Encoding {
    pub const fn as_str(self) -> &'static str {
        match self {
            Encoding::Cl100kBase => "cl100k_base",
            Encoding::P50kBase => "p50k_base",
            Encoding::R50kBase => "r50k_base",
            Encoding::O200kBase => "o200k_base",
        }
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Encoding {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "cl100k_base" => Ok(Encoding::Cl100kBase),
            "p50k_base" => Ok(Encoding::P50kBase),
            "r50k_base" => Ok(Encoding::R50kBase),
            "o200k_base" => Ok(Encoding::O200kBase),
            other => Err(format!(
                "unknown encoding {other:?}; expected one of cl100k_base, p50k_base, r50k_base, o200k_base"
            )),
        }
    }
}

/// Splits the `--models` value into individual IDs.
///
/// Trims surrounding whitespace and drops empty segments, so `-m "a, ,b,"`
/// yields `["a", "b"]` instead of three entries with blanks.
pub fn split_models(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Args {
    /// Model IDs to price, deduplicated while preserving command-line order so
    /// the table always reads the way the user typed it.
    pub fn model_list(&self) -> Vec<String> {
        let mut seen = Vec::new();
        let mut out = Vec::new();
        for id in split_models(&self.models) {
            if !seen.contains(&id) {
                seen.push(id.clone());
                out.push(id);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Args::command().debug_assert();
    }

    #[test]
    fn defaults_match_spec() {
        let args = Args::try_parse_from(["token-tax"]).unwrap();
        assert_eq!(args.encoding, Encoding::Cl100kBase);
        assert_eq!(args.output_tokens, DEFAULT_OUTPUT_TOKENS);
        assert_eq!(args.output_tokens, 1000);
        assert_eq!(args.models, DEFAULT_MODELS);
    }

    /// The default set must resolve against the offline snapshot. If it does not,
    /// a first run with no network shows three UNKNOWN rows and looks broken.
    #[test]
    fn default_models_all_exist_in_the_snapshot() {
        let catalog = crate::pricing::snapshot();
        for id in split_models(DEFAULT_MODELS) {
            assert!(
                catalog.iter().any(|m| m.id == id),
                "default model {id} is missing from the fallback snapshot"
            );
        }
    }

    #[test]
    fn default_models_are_all_qualified_openrouter_ids() {
        for id in split_models(DEFAULT_MODELS) {
            assert!(id.contains('/'), "default model {id} needs a vendor prefix");
        }
    }

    #[test]
    fn short_and_long_flags_agree() {
        let short = Args::try_parse_from(["token-tax", "-m", "a/b", "-o", "5", "-e", "o200k_base"])
            .unwrap();
        let long = Args::try_parse_from([
            "token-tax",
            "--models",
            "a/b",
            "--output-tokens",
            "5",
            "--encoding",
            "o200k_base",
        ])
        .unwrap();
        assert_eq!(short.models, long.models);
        assert_eq!(short.output_tokens, long.output_tokens);
        assert_eq!(short.encoding, long.encoding);
    }

    #[test]
    fn split_trims_and_drops_empties() {
        assert_eq!(split_models(" a , b ,, c ,"), vec!["a", "b", "c"]);
        assert_eq!(split_models("solo"), vec!["solo"]);
        assert!(split_models("").is_empty());
        assert!(split_models("  ,  ").is_empty());
    }

    #[test]
    fn model_list_dedupes_preserving_order() {
        let args = Args {
            models: "z/b,a/b,z/b".into(),
            output_tokens: 1,
            encoding: Encoding::Cl100kBase,
        };
        assert_eq!(args.model_list(), vec!["z/b", "a/b"]);
    }

    #[test]
    fn every_encoding_round_trips_through_str() {
        for enc in Encoding::ALL {
            assert_eq!(<Encoding as FromStr>::from_str(enc.as_str()).unwrap(), enc);
            assert_eq!(enc.to_string(), enc.as_str());
        }
    }

    /// Guards the one place where the two parsers can silently diverge: clap's
    /// `ValueEnum` kebab-cases variant names unless told otherwise, which would
    /// have turned `-e o200k_base` into an error.
    #[test]
    fn clap_accepts_the_same_spellings_as_from_str() {
        use clap::ValueEnum;
        for enc in Encoding::ALL {
            let value = enc.to_possible_value().expect("every variant is a value");
            assert_eq!(
                value.get_name(),
                enc.as_str(),
                "clap name drifted for {enc}"
            );
            assert!(
                value.get_name_and_aliases().count() <= 1,
                "{enc} should not need clap aliases"
            );

            // And the flag actually parses with the documented spelling.
            let parsed = Args::try_parse_from(["token-tax", "-e", enc.as_str()])
                .unwrap_or_else(|e| panic!("-e {enc} rejected: {e}"));
            assert_eq!(parsed.encoding, enc);
        }
    }

    #[test]
    fn kebab_case_is_rejected() {
        assert!(Args::try_parse_from(["token-tax", "-e", "o200k-base"]).is_err());
    }

    #[test]
    fn unknown_encoding_is_rejected_at_parse_time() {
        let err = Args::try_parse_from(["token-tax", "-e", "bogus"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
    }

    #[test]
    fn output_tokens_zero_is_allowed() {
        // Pricing only the prompt is a legitimate question.
        assert_eq!(
            Args::try_parse_from(["token-tax", "-o", "0"])
                .unwrap()
                .output_tokens,
            0
        );
    }

    #[test]
    fn non_numeric_output_tokens_is_rejected() {
        assert!(Args::try_parse_from(["token-tax", "-o", "many"]).is_err());
    }
}
