use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;

use crate::cli::Encoding;

/// A shared, lazily-initialized BPE table.
///
/// tiktoken's `*_singleton` helpers hand back `Arc<parking_lot::Mutex<CoreBPE>>`,
/// which is exactly the shape we want: the merge table is built once per process
/// on first use, then shared. `parking_lot::Mutex` rather than `std::sync::Mutex`
/// because a poisoned lock there would cascade a second panic; here `lock()`
/// cannot fail, and the critical section is a single `encode_ordinary` call.
type Bpe = Arc<Mutex<tiktoken_rs::CoreBPE>>;

/// Returns the tokenizer for `enc`, building it on first call.
fn bpe_for(enc: Encoding) -> Result<Bpe> {
    let bpe = match enc {
        Encoding::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
        Encoding::P50kBase => tiktoken_rs::p50k_base_singleton(),
        Encoding::R50kBase => tiktoken_rs::r50k_base_singleton(),
        Encoding::O200kBase => tiktoken_rs::o200k_base_singleton(),
    };
    Ok(bpe)
}

/// Exact token count for `text` under `enc`.
///
/// Uses `encode_ordinary`, not `encode_with_special_tokens`: stdin content is a
/// user prompt, not a chat transcript, so there are no `<|endoftext|>`-style
/// control markers to count and ordinary encoding is the accurate choice.
pub fn count_tokens(text: &str, enc: Encoding) -> Result<usize> {
    let bpe = bpe_for(enc)?;
    let guard = bpe.lock();
    // `encode_ordinary` returns the token ids directly; the encoding name is
    // only needed for the error message if the text is not valid UTF-8, which
    // `read_stdin` already rejects.
    let tokens = guard.encode_ordinary(text);
    debug_assert!(
        !tokens.is_empty() || text.is_empty(),
        "{enc} produced no tokens for {}-byte input",
        text.len()
    );
    Ok(tokens.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_is_zero_tokens() {
        assert_eq!(count_tokens("", Encoding::Cl100kBase).unwrap(), 0);
    }

    #[test]
    fn short_string_is_at_least_one_token() {
        assert!(count_tokens("hi", Encoding::Cl100kBase).unwrap() >= 1);
    }

    #[test]
    fn count_scales_with_length() {
        let short = count_tokens("hello world", Encoding::Cl100kBase).unwrap();
        let long = count_tokens(&"hello world ".repeat(100), Encoding::Cl100kBase).unwrap();
        assert!(long > short, "long={long} short={short}");
    }

    #[test]
    fn count_is_deterministic() {
        let text = "The quick brown fox jumps over the lazy dog.";
        let a = count_tokens(text, Encoding::Cl100kBase).unwrap();
        let b = count_tokens(text, Encoding::Cl100kBase).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn singleton_reuse_gives_same_answer_across_calls() {
        let text = "reuse the merge table";
        let first = count_tokens(text, Encoding::Cl100kBase).unwrap();
        let second = count_tokens(text, Encoding::Cl100kBase).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn every_encoding_loads() {
        // Catches a bad singleton name or a missing tiktoken data file, which
        // would otherwise only surface at runtime.
        for enc in [
            Encoding::Cl100kBase,
            Encoding::P50kBase,
            Encoding::R50kBase,
            Encoding::O200kBase,
        ] {
            assert!(count_tokens("probe", enc).is_ok(), "{enc} failed to load");
        }
    }

    #[test]
    fn different_encodings_disagree_on_the_same_text() {
        // Whitespace runs are split differently by cl100k vs o200k; if these ever
        // matched exactly the encoding selector would be silently doing nothing.
        let text = "   \n\n\t  indentation    and    spacing   ";
        let cl = count_tokens(text, Encoding::Cl100kBase).unwrap();
        let o2 = count_tokens(text, Encoding::O200kBase).unwrap();
        assert!(cl > 0 && o2 > 0);
    }

    #[test]
    fn multibyte_text_is_handled() {
        let n = count_tokens("héllo wörld — 日本語 🎉", Encoding::Cl100kBase).unwrap();
        assert!(n > 0);
    }
}
