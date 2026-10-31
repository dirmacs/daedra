//! Relevance ranking, URL canonicalisation, and query tokenisation.

pub(super) fn canonical_url_key(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let mut path = u.path().trim_end_matches('/').to_string();
    if path.is_empty() {
        path.push('/');
    }
    let kept: Vec<String> = u
        .query_pairs()
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            !(k.starts_with("utm_")
                || matches!(
                    k.as_ref(),
                    "fbclid" | "gclid" | "ref" | "spm" | "mc_cid" | "mc_eid"
                ))
        })
        .map(|(k, v)| format!("{}={}", urlencoding::encode(&k), urlencoding::encode(&v)))
        .collect();
    let query = if kept.is_empty() {
        String::new()
    } else {
        format!("?{}", kept.join("&"))
    };
    Some(format!("{host}{path}{query}"))
}

/// Words too common to carry query meaning. Kept deliberately small: a
/// missed stopword only costs a little precision; a wrongly listed content
/// word costs recall.
fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "an"
            | "of"
            | "in"
            | "on"
            | "for"
            | "to"
            | "and"
            | "or"
            | "is"
            | "are"
            | "was"
            | "were"
            | "be"
            | "at"
            | "by"
            | "it"
            | "its"
            | "with"
            | "from"
            | "as"
            | "that"
            | "this"
            | "how"
            | "what"
            | "why"
            | "do"
            | "does"
            | "did"
            | "my"
            | "me"
            | "you"
            | "your"
            | "he"
            | "she"
            | "we"
            | "they"
            | "not"
            | "no"
            | "de"
            | "la"
    )
}

/// Lowercase significant tokens of a query: alphanumeric runs of length 2+,
/// stopwords removed.
pub(super) fn query_tokens(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2 && !is_stopword(t))
        .map(str::to_string)
        .collect()
}

/// True when `hay` contains a leading fragment of `token` at least half the
/// token long (minimum four characters). Search engines answer rare names
/// with stem matches ("baalateja" → "Baalat Gebal"); the literal token never
/// appears, so pure containment scoring rejects every result. The half-token
/// bar keeps keyboard-mash fragments ("asdf" inside "asdfqwertzxcv") from
/// counting: close-name stems pass, coincidental slivers do not.
fn partial_hit(token: &str, hay: &str) -> bool {
    let min = 4;
    if token.len() <= min {
        return false;
    }
    let need = token.len().div_ceil(2).max(min);
    let max = token.len() - 1;
    (need..=max).rev().any(|l| hay.contains(&token[..l]))
}

/// Share of query tokens present in a result's title, URL, or description.
/// The URL counts — a domain like `tokio.rs` should match "tokio". An empty
/// token list scores everything neutral (1.0), which keeps the k-way merge
/// order equivalent to the old round-robin when the query carries no signal.
/// A token whose leading fragment appears in the result counts half.
pub(super) fn relevance_score(r: &crate::types::SearchResult, tokens: &[String]) -> f64 {
    if tokens.is_empty() {
        return 1.0;
    }
    let title_hay = format!("{} {}", r.title, r.url).to_lowercase();
    let hay = format!("{title_hay} {}", r.description).to_lowercase();
    let mut score = 0.0;
    for t in tokens {
        if hay.contains(t.as_str()) {
            score += 1.0;
        } else if partial_hit(t, &hay) {
            score += 0.5;
        }
    }
    score /= tokens.len() as f64;
    // Full-phrase bonus: the query words in order ("capital ... france" in
    // "Capital of France") outrank a story that merely contains both words.
    if tokens_in_order(&title_hay, tokens) {
        score += 1.0;
    }
    score
}

/// True when every token appears in `hay`, in order, with anything (or
/// nothing) between them.
fn tokens_in_order(hay: &str, tokens: &[String]) -> bool {
    let mut from = 0;
    for t in tokens {
        match hay[from..].find(t.as_str()) {
            Some(p) => from += p + t.len(),
            None => return false,
        }
    }
    true
}

