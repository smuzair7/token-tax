use anyhow::Result;
use serde::Deserialize;

/// Where the pricing numbers came from, so the UI can be honest about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Fetched from OpenRouter during this run.
    Live,
    /// Compiled-in snapshot; the network was unavailable or the request failed.
    Snapshot,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Live => "live",
            Source::Snapshot => "offline snapshot",
        }
    }
}

const ENDPOINT: &str = "https://openrouter.ai/api/v1/models";

/// Hand-maintained fallback prices, used when OpenRouter is unreachable.
///
/// **Values are USD per single token**, matching OpenRouter's wire format, so
/// this table and the live path can never disagree about scale. Divide by
/// 1_000_000 to get the familiar "$/Mtok" figure.
///
/// These are list prices transcribed at authoring time and will drift. That is
/// the intended trade-off: the tool stays useful offline and flags the data as
/// stale rather than pretending the numbers are current.
const FALLBACK: &[(&str, f64, f64, Option<u64>)] = &[
    ("openai/gpt-4o", 2.500e-6, 1.000e-5, Some(128_000)),
    ("openai/gpt-4o-mini", 1.500e-7, 6.000e-7, Some(128_000)),
    ("openai/gpt-4.1", 2.000e-6, 8.000e-6, Some(1_047_576)),
    ("openai/gpt-4.1-mini", 4.000e-7, 1.600e-6, Some(1_047_576)),
    ("openai/gpt-4-turbo", 1.000e-5, 3.000e-5, Some(128_000)),
    ("openai/gpt-4", 3.000e-5, 6.000e-5, Some(8_191)),
    ("openai/gpt-3.5-turbo", 5.000e-7, 1.500e-6, Some(16_385)),
    ("openai/o1", 1.500e-5, 6.000e-5, Some(200_000)),
    (
        "anthropic/claude-sonnet-4.6",
        3.000e-6,
        1.500e-5,
        Some(1_000_000),
    ),
    (
        "anthropic/claude-haiku-4.5",
        1.000e-6,
        5.000e-6,
        Some(200_000),
    ),
    (
        "anthropic/claude-sonnet-4",
        3.000e-6,
        1.500e-5,
        Some(200_000),
    ),
    ("google/gemini-2.5-pro", 1.250e-6, 1.000e-5, Some(1_048_576)),
    (
        "google/gemini-2.5-flash",
        3.000e-7,
        2.500e-6,
        Some(1_048_576),
    ),
    (
        "google/gemini-2.5-flash-lite",
        1.000e-7,
        4.000e-7,
        Some(1_048_576),
    ),
    (
        "meta-llama/llama-3.3-70b-instruct",
        1.000e-7,
        3.200e-7,
        Some(131_072),
    ),
    ("mistralai/mistral-large", 2.000e-6, 6.000e-6, Some(128_000)),
    ("deepseek/deepseek-chat", 2.574e-7, 1.029e-6, Some(163_840)),
    ("qwen/qwen3-235b-a22b", 4.550e-7, 1.820e-6, Some(131_072)),
];

#[derive(Clone, Debug)]
pub struct ModelPrice {
    pub id: String,
    /// USD per input token.
    pub prompt_per_token: f64,
    /// USD per output token.
    pub completion_per_token: f64,
    /// Total context window in tokens; `None` when OpenRouter omitted it.
    pub context_length: Option<u64>,
}

impl ModelPrice {
    pub fn prompt_per_mtok(&self) -> f64 {
        self.prompt_per_token * 1_000_000.0
    }

    pub fn completion_per_mtok(&self) -> f64 {
        self.completion_per_token * 1_000_000.0
    }

    pub fn input_cost(&self, input_tokens: u64) -> f64 {
        input_tokens as f64 * self.prompt_per_token
    }

    pub fn output_cost(&self, output_tokens: u64) -> f64 {
        output_tokens as f64 * self.completion_per_token
    }

    pub fn total_cost(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        self.input_cost(input_tokens) + self.output_cost(output_tokens)
    }

    /// Whether the assumed token counts can actually fit in the context window.
    pub fn fits(&self, input_tokens: u64, output_tokens: u64) -> bool {
        match self.context_length {
            Some(limit) => input_tokens.saturating_add(output_tokens) <= limit,
            None => true,
        }
    }
}

/// A requested model, resolved against the catalog.
#[derive(Clone, Debug)]
pub struct Row {
    /// The ID exactly as the user typed it.
    pub requested: String,
    pub price: Option<ModelPrice>,
    /// Nearest catalog IDs, populated only when resolution failed.
    pub suggestions: Vec<String>,
}

impl Row {
    pub fn is_unknown(&self) -> bool {
        self.price.is_none()
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default, deserialize_with = "null_as_default")]
    data: Vec<RawModel>,
}

#[derive(Debug, Deserialize)]
struct RawModel {
    id: String,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    pricing: Option<RawPricing>,
}

#[derive(Debug, Deserialize)]
struct RawPricing {
    /// USD per token. OpenRouter sends a decimal *string* ("0.000003"); some
    /// entries send a bare JSON number, so accept both.
    #[serde(default, deserialize_with = "null_as_default")]
    prompt: Option<Price>,
    #[serde(default, deserialize_with = "null_as_default")]
    completion: Option<Price>,
}

/// A price that may arrive as `"0.000003"` or `0.000003`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Price {
    Num(f64),
    Text(String),
}

impl Price {
    fn as_f64(&self) -> Option<f64> {
        let value = match self {
            Price::Num(n) => *n,
            Price::Text(s) => s.trim().parse().ok()?,
        };
        (value.is_finite() && value >= 0.0).then_some(value)
    }
}

/// Treats an explicit `null` as an absent field, so `{"data": null}` and a
/// missing `pricing` both land on `Default` instead of a deserialization error.
fn null_as_default<'de, D, T>(de: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(de)?.unwrap_or_default())
}

impl ModelsResponse {
    /// Flattens the wire format into `ModelPrice`s.
    ///
    /// Entries without usable numeric pricing are skipped rather than defaulted,
    /// so a free-but-unspecified model shows up as UNKNOWN instead of a
    /// fabricated $0.00 that would quietly win every comparison.
    fn into_prices(self) -> Vec<ModelPrice> {
        self.data
            .into_iter()
            .filter_map(|raw| {
                let pricing = raw.pricing?;
                let prompt = pricing.prompt?.as_f64()?;
                let completion = pricing.completion?.as_f64()?;
                Some(ModelPrice {
                    id: raw.id,
                    prompt_per_token: prompt,
                    completion_per_token: completion,
                    context_length: raw.context_length,
                })
            })
            .collect()
    }
}

pub fn snapshot() -> Vec<ModelPrice> {
    FALLBACK
        .iter()
        .map(|&(id, prompt, completion, context_length)| ModelPrice {
            id: id.to_string(),
            prompt_per_token: prompt,
            completion_per_token: completion,
            context_length,
        })
        .collect()
}

/// Fetches live pricing, falling back to the compiled-in snapshot.
///
/// Every failure mode collapses to `Err` carrying the reason; the caller decides
/// to warn and continue. This function never returns an empty catalog, because an
/// empty catalog would render a table of nothing.
pub async fn fetch_or_snapshot(
    timeout: std::time::Duration,
) -> Result<(Vec<ModelPrice>, Source), String> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("token-tax/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("could not build HTTP client: {e}"))?;

    match client.get(ENDPOINT).send().await {
        Ok(resp) if resp.status().is_success() => {
            let body = resp
                .text()
                .await
                .map_err(|e| format!("could not read response body: {e}"))?;
            let parsed: ModelsResponse = serde_json::from_str(&body)
                .map_err(|e| format!("unexpected response from OpenRouter: {e}"))?;
            let prices = parsed.into_prices();
            if prices.is_empty() {
                Err("OpenRouter returned no priced models".to_string())
            } else {
                Ok((prices, Source::Live))
            }
        }
        Ok(resp) => Err(format!(
            "OpenRouter returned HTTP {}",
            resp.status().as_u16()
        )),
        Err(e) if e.is_timeout() => Err(format!("request timed out after {timeout:?}")),
        Err(e) if e.is_connect() => Err("could not reach openrouter.ai (offline?)".to_string()),
        Err(e) => Err(format!("network error: {e}")),
    }
}

/// Resolves requested IDs against the catalog.
///
/// Exact (case-insensitive) match wins. Otherwise the closest few IDs by
/// Jaro-Winkler similarity become suggestions on an UNKNOWN row.
pub fn resolve(requested: &[String], catalog: &[ModelPrice]) -> Vec<Row> {
    requested
        .iter()
        .map(|id| {
            if let Some(found) = catalog.iter().find(|m| m.id.eq_ignore_ascii_case(id)) {
                return Row {
                    requested: id.clone(),
                    price: Some(found.clone()),
                    suggestions: Vec::new(),
                };
            }
            Row {
                requested: id.clone(),
                price: None,
                suggestions: suggestions_for(id, catalog),
            }
        })
        .collect()
}

/// Scores how likely `id` is what the user meant by `query`, or `None` if the
/// pair is too dissimilar to be worth suggesting.
///
/// Two signals, because neither alone works on OpenRouter IDs:
///
/// 1. **Substring containment.** The dominant real-world error is dropping the
///    `vendor/` prefix (`gpt-4o` for `openai/gpt-4o`). Jaro-Winkler is
///    unreliable here because it divides by length, so a short query scores
///    *worse* than its own match. Containment is a much stronger signal, scaled
///    by how much of the ID the query covers so the tightest match ranks first.
/// 2. **Jaro-Winkler, for genuine typos** (`gpt-4o-minni`). Gated on length
///    ratio, otherwise a three-character query matches any long ID that happens
///    to contain those characters.
///
/// Comparisons are case-insensitive; the catalog uses lowercase, users do not
/// reliably.
fn similarity(query: &str, id: &str) -> Option<f64> {
    let q = query.to_lowercase();
    let i = id.to_lowercase();
    let q_len = q.chars().count();
    let i_len = i.chars().count();

    // --- 1. substring containment -----------------------------------------
    // Guarded by a minimum query length: a one- or two-character query is
    // contained in a large fraction of the catalog and would produce noise.
    if q_len >= MIN_SUBSTRING_QUERY_CHARS && i.contains(&q) {
        let coverage = q_len as f64 / i_len as f64;
        // 0.88 at near-zero coverage rising toward 0.99 as the query covers
        // more of the ID, so containment always outranks a mere typo.
        let mut score = 0.88 + 0.10 * coverage;

        // Matching the model half rather than the vendor half means the user
        // named the model they wanted.
        if let Some((_, model)) = i.rsplit_once('/') {
            if model.contains(&q) {
                score += 0.02;
            }
        }

        // Starting at a segment boundary reads as a deliberate truncation
        // rather than an accidental substring.
        let at_boundary = i[..i.find(&q).unwrap_or(0)]
            .chars()
            .next_back()
            .map_or(true, |c| c == '/' || c == '-');
        if at_boundary {
            score += 0.01;
        }

        return Some(score.min(0.999));
    }

    // --- 2. typo similarity -------------------------------------------------
    // Require comparable lengths. Without this, `gpt` matches
    // `gryphe/mythomax-l2-13b` (ratio 0.14) while `gpt-4o` fails to match
    // `openai/gpt-4o` (ratio 0.46) - exactly backwards.
    let ratio = q_len.min(i_len) as f64 / q_len.max(i_len).max(1) as f64;
    if ratio < MIN_LENGTH_RATIO {
        return None;
    }

    let jw = jaro_winkler(query, id);
    (jw >= MIN_SIMILARITY).then_some(jw)
}

/// Shortest query that may be matched by containment. Below this, a query is
/// too short to carry intent.
const MIN_SUBSTRING_QUERY_CHARS: usize = 3;

/// Minimum length ratio for the typo path.
const MIN_LENGTH_RATIO: f64 = 0.5;

/// Minimum Jaro-Winkler score for the typo path.
const MIN_SIMILARITY: f64 = 0.72;

/// Top `MAX_SUGGESTIONS` catalog IDs similar to `query`.
fn suggestions_for(query: &str, catalog: &[ModelPrice]) -> Vec<String> {
    const MAX_SUGGESTIONS: usize = 3;

    let mut scored: Vec<(f64, &str)> = catalog
        .iter()
        .filter_map(|m| similarity(query, &m.id).map(|s| (s, m.id.as_str())))
        .collect();

    // Ties are common when several IDs contain the query (every `gpt-4o`
    // variant contains `gpt-4o`). Prefer the shorter ID: it is the least
    // qualified name and therefore the least surprising suggestion.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.len().cmp(&b.1.len()))
    });

    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, id)| id.to_string())
        .collect()
}

/// Jaro-Winkler similarity in `[0.0, 1.0]`.
///
/// Prefers models whose first characters agree, which is what makes it good at
/// catching the common `claude-3.5-sonnet` -> `anthropic/claude-3.5-sonnet`
/// omission. Defined on chars rather than bytes so multi-byte vendor names do
/// not get sliced mid-codepoint.
fn jaro_winkler(a: &str, b: &str) -> f64 {
    let x: Vec<char> = a.chars().collect();
    let y: Vec<char> = b.chars().collect();

    if x.is_empty() && y.is_empty() {
        return 1.0;
    }
    if x.is_empty() || y.is_empty() {
        return 0.0;
    }
    if x == y {
        return 1.0;
    }

    // Standard match window: half the longer string, rounded down, min 1.
    let window = (x.len().max(y.len()) / 2).saturating_sub(1).max(1);

    let mut x_matched = vec![false; x.len()];
    let mut y_matched = vec![false; y.len()];
    let mut matches = 0usize;

    for (i, xc) in x.iter().enumerate() {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(y.len());
        for j in lo..hi {
            if !y_matched[j] && y[j] == *xc {
                x_matched[i] = true;
                y_matched[j] = true;
                matches += 1;
                break;
            }
        }
    }

    if matches == 0 {
        return 0.0;
    }

    // Count transpositions: walk the matched subsequences in order and count
    // positions where they disagree.
    let mut transpositions = 0usize;
    let mut k = 0usize;
    for (i, xc) in x.iter().enumerate() {
        if !x_matched[i] {
            continue;
        }
        while !y_matched[k] {
            k += 1;
        }
        if *xc != y[k] {
            transpositions += 1;
        }
        k += 1;
    }
    let transpositions = transpositions / 2;

    let m = matches as f64;
    let jaro = (m / x.len() as f64 + m / y.len() as f64 + (m - transpositions as f64) / m) / 3.0;

    // Winkler bonus, capped at 4 leading characters as in the original paper.
    let prefix = x
        .iter()
        .zip(y.iter())
        .take(4)
        .take_while(|(p, q)| p == q)
        .count();
    let jaro = jaro.clamp(0.0, 1.0);
    jaro + prefix as f64 * 0.1 * (1.0 - jaro)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp2(id: &str, prompt: f64, completion: f64) -> ModelPrice {
        ModelPrice {
            id: id.to_string(),
            prompt_per_token: prompt,
            completion_per_token: completion,
            context_length: None,
        }
    }

    // ---- price parsing -----------------------------------------------------

    fn parse_str(s: &str) -> Option<f64> {
        serde_json::from_str::<Price>(s).ok()?.as_f64()
    }

    #[test]
    fn parses_documented_price_string() {
        // From the spec: "0.000003" means $3 per million tokens.
        assert_eq!(parse_str(r#""0.000003""#), Some(3e-6));
        assert_eq!(parse_str(r#""0.000015""#), Some(1.5e-5));
    }

    #[test]
    fn parses_zero_and_whitespace() {
        assert_eq!(parse_str(r#""0""#), Some(0.0));
        assert_eq!(parse_str(r#"" 0.000003 ""#), Some(3e-6));
    }

    #[test]
    fn rejects_garbage_and_negative() {
        assert_eq!(parse_str(r#""""#), None);
        assert_eq!(parse_str(r#""free""#), None);
        assert_eq!(parse_str(r#""NaN""#), None);
        assert_eq!(parse_str(r#""-1.0""#), None);
        assert_eq!(parse_str(r#""inf""#), None);
        assert_eq!(parse_str("null"), None);
        assert_eq!(parse_str("{}"), None);
    }

    #[test]
    fn accepts_bare_numbers() {
        assert_eq!(parse_str("0.000003"), Some(3e-6));
        assert_eq!(parse_str("-1.0"), None);
        assert_eq!(parse_str("1e999"), None, "non-finite must be rejected");
    }

    // ---- JSON --------------------------------------------------------------

    #[test]
    fn parses_spec_response_shape() {
        let json = r#"{
            "data": [
                {"id": "anthropic/claude-3.5-sonnet",
                 "context_length": 200000,
                 "pricing": {"prompt": "0.000003", "completion": "0.000015"}}
            ]
        }"#;
        let prices = serde_json::from_str::<ModelsResponse>(json)
            .unwrap()
            .into_prices();
        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].id, "anthropic/claude-3.5-sonnet");
        assert_eq!(prices[0].prompt_per_mtok(), 3.0);
        assert_eq!(prices[0].completion_per_mtok(), 15.0);
        assert_eq!(prices[0].context_length, Some(200_000));
    }

    /// A model is only usable when *both* halves of its price are known. Half a
    /// price cannot be defaulted without inventing a number, so such entries are
    /// dropped and surface as UNKNOWN rows rather than as a wrong cost.
    #[test]
    fn drops_models_missing_either_half_of_the_price() {
        let json = r#"{"data":[
            {"id": "good/model", "pricing": {"prompt": "0.000001", "completion": "0.000002"}},
            {"id": "null-prompt/model", "pricing": {"prompt": null, "completion": "0.000001"}},
            {"id": "no-pricing/model"},
            {"id": "null-pricing/model", "pricing": null},
            {"id": "only-prompt/model", "pricing": {"prompt": "0.000001"}},
            {"id": "only-completion/model", "pricing": {"completion": "0.000001"}},
            {"id": "garbage/model", "pricing": {"prompt": "free", "completion": "0.000001"}}
        ]}"#;
        let prices = serde_json::from_str::<ModelsResponse>(json)
            .unwrap()
            .into_prices();
        assert_eq!(
            prices.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["good/model"]
        );
    }

    #[test]
    fn tolerates_empty_and_null_data() {
        for json in [r#"{"data":[]}"#, r#"{"data":null}"#, "{}"] {
            let parsed = serde_json::from_str::<ModelsResponse>(json).unwrap();
            assert!(parsed.into_prices().is_empty());
        }
    }

    #[test]
    fn numeric_prices_as_json_numbers_also_work() {
        // Defensive: some providers return unquoted numbers.
        let json = r#"{"data":[{"id":"a/b","pricing":{"prompt":0.000003,"completion":0.000015}}]}"#;
        let prices = serde_json::from_str::<ModelsResponse>(json)
            .unwrap()
            .into_prices();
        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].prompt_per_mtok(), 3.0);
    }

    // ---- cost arithmetic ---------------------------------------------------

    #[test]
    fn cost_math_is_correct() {
        let m = mp2("a/b", 3e-6, 1.5e-5);
        assert!((m.input_cost(1_000_000) - 3.0).abs() < 1e-9);
        assert!((m.output_cost(1_000_000) - 15.0).abs() < 1e-9);
        assert!((m.total_cost(1_000, 1_000) - 0.018).abs() < 1e-12);
    }

    #[test]
    fn zero_output_tokens_prices_prompt_only() {
        let m = mp2("a/b", 3e-6, 1.5e-5);
        assert!((m.total_cost(1_000, 0) - 0.003).abs() < 1e-12);
    }

    #[test]
    fn context_fit_flags_overflow() {
        let mut m = mp2("a/b", 3e-6, 1.5e-5);
        m.context_length = Some(1_000);
        assert!(!m.fits(900, 200));
        assert!(m.fits(800, 200));
        m.context_length = None;
        assert!(m.fits(u64::MAX, u64::MAX));
    }

    // ---- resolution --------------------------------------------------------

    #[test]
    fn exact_match_resolves_without_suggestions() {
        let catalog = vec![mp2("anthropic/claude-3.5-sonnet", 3e-6, 1.5e-5)];
        let rows = resolve(&["anthropic/claude-3.5-sonnet".into()], &catalog);
        assert!(!rows[0].is_unknown());
        assert!(rows[0].suggestions.is_empty());
    }

    #[test]
    fn match_is_case_insensitive_but_keeps_requested_spelling() {
        let catalog = vec![mp2("OpenAI/GPT-4o", 2.5e-6, 1e-5)];
        let rows = resolve(&["openai/gpt-4o".into()], &catalog);
        assert!(!rows[0].is_unknown());
        assert_eq!(rows[0].requested, "openai/gpt-4o");
    }

    /// The most common user error is dropping the vendor prefix. This asserts the
    /// suggestion machinery recovers a real catalog ID from that mistake.
    #[test]
    fn missing_vendor_prefix_suggests_the_real_id() {
        let catalog = snapshot();
        let rows = resolve(&["claude-sonnet-4".into()], &catalog);
        assert!(rows[0].is_unknown());
        assert!(
            rows[0]
                .suggestions
                .contains(&"anthropic/claude-sonnet-4".to_string()),
            "got {:?}",
            rows[0].suggestions
        );
    }

    /// A plain typo in the model half should also surface the intended model.
    #[test]
    fn typo_suggests_the_intended_id() {
        let catalog = snapshot();
        let rows = resolve(&["openai/gpt-4o-mini-x".into()], &catalog);
        assert!(rows[0].is_unknown());
        assert!(
            rows[0]
                .suggestions
                .contains(&"openai/gpt-4o-mini".to_string()),
            "got {:?}",
            rows[0].suggestions
        );
    }

    #[test]
    fn resolution_preserves_request_order_and_length() {
        let catalog = snapshot();
        let want = vec!["openai/gpt-4o".to_string(), "nope/nope".to_string()];
        let rows = resolve(&want, &catalog);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].requested, want[0]);
        assert_eq!(rows[1].requested, want[1]);
    }

    // ---- similarity --------------------------------------------------------

    #[test]
    fn jaro_winkler_basics() {
        assert!((jaro_winkler("abc", "abc") - 1.0).abs() < 1e-9);
        assert_eq!(jaro_winkler("", "abc"), 0.0);
        assert_eq!(jaro_winkler("abc", ""), 0.0);
        assert_eq!(jaro_winkler("", ""), 1.0);
        assert_eq!(jaro_winkler("abc", "xyz"), 0.0);
    }

    #[test]
    fn jaro_winkler_ranks_typo_above_unrelated() {
        let near = jaro_winkler("gpt-4o-mini", "gpt-4o-minni");
        let far = jaro_winkler("gpt-4o-mini", "llama-3.1-405b");
        assert!(near > far, "near={near} far={far}");
    }

    #[test]
    fn jaro_winkler_uses_prefix_bonus() {
        // Same multiset, but only one version shares a leading prefix.
        let prefix_bonus = jaro_winkler("anthropic/sonnet", "anthropic/claude");
        let no_bonus = jaro_winkler("zzz/claude", "anthropic/claude");
        assert!(prefix_bonus > no_bonus);
    }

    #[test]
    fn suggestions_reject_unrelated_queries() {
        assert!(suggestions_for("zzzzzzzzzzzz", &snapshot()).is_empty());
    }

    // ---- regression: the length-bias defect ---------------------------------
    //
    // Jaro-Winkler divides by string length, and every OpenRouter ID carries a
    // `vendor/` prefix the user often omits. That made the real match score
    // *below* the threshold while an unrelated long ID sharing three scattered
    // characters scored above it.

    #[test]
    fn dropped_vendor_prefix_outranks_typo_scoring() {
        let catalog = snapshot();
        // Regression: scored 0.329 against its own match, below the 0.72 gate.
        let rows = resolve(&["gpt-4o".into()], &catalog);
        assert_eq!(
            rows[0].suggestions.first().map(String::as_str),
            Some("openai/gpt-4o"),
            "got {:?}",
            rows[0].suggestions
        );
    }

    #[test]
    fn bare_family_name_suggests_that_family() {
        let catalog = snapshot();
        let rows = resolve(&["gemini-2.5-flash".into()], &catalog);
        assert!(
            rows[0]
                .suggestions
                .iter()
                .any(|s| s.contains("gemini-2.5-flash")),
            "got {:?}",
            rows[0].suggestions
        );
    }

    #[test]
    fn short_query_cannot_match_an_unrelated_long_id() {
        // Regression: `gpt` scored 0.741 against `gryphe/mythomax-l2-13b` and
        // was suggested. Every suggestion must be a real model of that family.
        let catalog = snapshot();
        let rows = resolve(&["gpt".into()], &catalog);
        for s in &rows[0].suggestions {
            assert!(
                s.contains("gpt"),
                "short query suggested unrelated model {s:?}: {:?}",
                rows[0].suggestions
            );
        }
    }

    #[test]
    fn single_and_double_char_queries_get_no_suggestions() {
        let catalog = snapshot();
        for q in ["x", "a", "o", "-"] {
            let rows = resolve(&[q.to_string()], &catalog);
            assert!(
                rows[0].suggestions.is_empty(),
                "{q:?} produced noise: {:?}",
                rows[0].suggestions
            );
        }
    }

    #[test]
    fn genuine_typos_still_resolve() {
        let catalog = snapshot();
        let rows = resolve(&["openai/gpt-4o-minni".into()], &catalog);
        assert_eq!(
            rows[0].suggestions.first().map(String::as_str),
            Some("openai/gpt-4o-mini")
        );
    }

    #[test]
    fn case_differences_do_not_defeat_suggestions() {
        let catalog = snapshot();
        let rows = resolve(&["GPT-4O".into()], &catalog);
        assert_eq!(
            rows[0].suggestions.first().map(String::as_str),
            Some("openai/gpt-4o")
        );
    }

    #[test]
    fn exact_match_never_gets_suggestions() {
        let catalog = snapshot();
        let rows = resolve(&["openai/gpt-4o".into()], &catalog);
        assert!(!rows[0].is_unknown());
        assert!(rows[0].suggestions.is_empty());
    }

    #[test]
    fn similarity_is_none_for_weak_pairs_and_some_for_strong_ones() {
        assert!(similarity("gpt-4o", "openai/gpt-4o").is_some());
        assert!(similarity("gpt", "gryphe/mythomax-l2-13b").is_none());
        assert!(similarity("x", "x-ai/grok-4.7").is_none(), "too short");
        assert!(similarity("zzzzzzzzzzzz", "openai/gpt-4o").is_none());
    }

    #[test]
    fn suggestion_is_ordered_most_similar_first() {
        let catalog = snapshot();
        let rows = resolve(&["gemini-2.5".into()], &catalog);
        let scores: Vec<f64> = rows[0]
            .suggestions
            .iter()
            .map(|s| similarity("gemini-2.5", s).unwrap())
            .collect();
        assert!(
            scores.windows(2).all(|w| w[0] >= w[1]),
            "suggestions not ranked by score: {:?} -> {scores:?}",
            rows[0].suggestions
        );
    }

    #[test]
    fn ties_prefer_the_shorter_model_id() {
        let catalog = snapshot();
        let rows = resolve(&["claude-sonnet-4".into()], &catalog);
        let lengths: Vec<usize> = rows[0].suggestions.iter().map(String::len).collect();
        assert!(
            lengths.windows(2).all(|w| w[0] <= w[1]),
            "tie-break failed, lengths {lengths:?} for {:?}",
            rows[0].suggestions
        );
    }

    // ---- snapshot ----------------------------------------------------------

    #[test]
    fn snapshot_ids_are_unique_and_qualified() {
        let snap = snapshot();
        // Wide enough to cover a default run plus the usual suspects.
        assert!(snap.len() >= 15, "snapshot has only {} models", snap.len());
        let mut ids: Vec<&str> = snap.iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate model id in FALLBACK");
        assert!(snap.iter().all(|m| m.id.contains('/')));
    }

    #[test]
    fn snapshot_prices_are_sane() {
        for m in snapshot() {
            assert!(m.prompt_per_token >= 0.0, "{}", m.id);
            assert!(m.completion_per_token >= 0.0, "{}", m.id);
            assert!(m.prompt_per_mtok() <= 1_000.0, "{}", m.id);
            assert!(m.context_length.is_some_and(|c| c > 0), "{}", m.id);
            // Output is priced at or above input on every current model.
            assert!(m.completion_per_token >= m.prompt_per_token, "{}", m.id);
        }
    }

    #[test]
    fn source_labels_differ() {
        assert_ne!(Source::Live.label(), Source::Snapshot.label());
    }
}
