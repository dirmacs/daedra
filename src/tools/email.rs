//! Contact-address extraction from a fetched page.
//!
//! Address discovery is the one thing an outreach agent needs from a page and
//! the one thing a generic text extractor destroys. Measured on this host
//! (2026-11-01), naïve `[a-z]+@[a-z]+\.[a-z]+` over rendered Markdown returns
//! four classes of wrong answer for every right one:
//!
//! | wrong answer | origin |
//! |---|---|
//! | `Emailbuild@dirmacs.com` | a link whose visible text "Email" abutted its `mailto:` target during HTML→Markdown conversion |
//! | `homepage@1440.webp` | an image filename (`foo@2x.png`) |
//! | `alice@example.com` | placeholder content in a demo or docs page |
//! | `mainmatter@fosstodon.org` | a Fediverse handle, which is not an address |
//!
//! A wrong address is worse than none: the outreach ledger holds
//! `leads.address_key UNIQUE` and `sends.lead_id UNIQUE`, so recording a
//! non-existent address spends the one slot that address will ever have and
//! makes the real one unsendable.
//!
//! # Where the two sources live
//!
//! Neither source can be recovered from extracted output, so this module must
//! be called while the inputs are still in hand:
//!
//! - **`mailto:` targets** exist only in raw HTML. `is_skippable_href`
//!   (`fetch/client.rs:167`, `crawl.rs:275`) drops `mailto:` from `links`
//!   before the caller ever sees it, and the raw `Html` document is discarded
//!   once [`PageContent`] is built. [`extract_from_html`] therefore takes the
//!   HTML string, not the parsed links.
//! - **Visible text** exists in the rendered Markdown, which is what a human
//!   reading the page would see. [`extract_from_text`] takes that.
//!
//! [`PageContent`]: crate::types::PageContent

use std::collections::BTreeSet;

use regex::Regex;
use std::sync::OnceLock;

/// One surviving address, with the source it was found in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Found {
    /// Lowercased address, as it should be stored.
    pub address: String,
}

impl Found {
    fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into().to_lowercase(),
        }
    }
}

/// Image/video/archive extensions that appear in `name@2x.ext` filenames.
///
/// `sprite@2x.png` is a valid match for an address shape; it is not an
/// address. Asset filenames are by far the most common false positive on
/// media-heavy marketing pages.
const ASSET_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "bmp", "ico", "tif", "tiff", "heic", "mp4",
    "webm", "mov", "mp3", "wav", "ogg", "pdf", "zip", "gz", "tar", "woff", "woff2", "ttf", "eot",
    "css", "js", "map", "json", "xml", "rss", "atom",
];

/// Reserved example/test domains (RFC 2606, RFC 6761).
///
/// `alice@example.com` and `user@test.invalid` are placeholders that appear in
/// documentation and demo content. Sending to them is guaranteed failure, and
/// recording them spends a ledger slot.
const PLACEHOLDER_DOMAINS: &[&str] = &[
    "example.com",
    "example.org",
    "example.net",
    "example.edu",
    "test.com",
    "test.invalid",
    "invalid",
    "localhost",
    "domain.com",
    "email.com",
    "yourdomain.com",
    "company.com",
    "mysite.com",
    "site.com",
];

/// Social/fediverse hosts that publish handles in `user@host` form.
///
/// These are ActivityPub identifiers, not mailboxes. Recorded as a distinct
/// class because they are otherwise indistinguishable from a real address —
/// a domain allowlist is the only reliable separator, and that is an
/// assessment-stage decision this module must not make silently.
const FEDERATION_HOSTS: &[&str] = &[
    "fosstodon.org",
    "mastodon.social",
    "mastodon.online",
    "hachyderm.io",
    "infosec.exchange",
    "techhub.social",
    "ruby.social",
    "federated.press",
    "threads.net",
    "bsky.social",
];

/// Matches an address-shaped substring, anchored on non-word boundaries so it
/// does not start mid-token.
fn address_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)(?:[a-z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-z0-9!#$%&'*+/=?^_`{|}~-]+)*)",
            r"@(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z]{2,}"
        ))
        .expect("address regex is valid")
    })
}

/// Matches a `mailto:` target in raw HTML, capturing the address.
///
/// Accepts the query-string form (`mailto:a@b.c?subject=...`) and stops at the
/// first parameter separator, quote, or tag close.
fn mailto_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)mailto:([^"'?>\s]+)"#).expect("mailto regex is valid"))
}

/// Resolve a run of candidates, dropping a glued-label variant when the
/// unglued address is also present in the same document.
///
/// A rendering pass glues an anchor's visible text onto its `mailto:` target:
/// `<a href="mailto:build@dirmacs.com">Email</a>` becomes
/// `Emailbuild@dirmacs.com`. The glue cannot be split reliably from the string
/// alone — `Emailbuild` is a plausible local part, and a Title-Case rule would
/// mangle a real `JohnSmith@acme.com`.
///
/// What *is* decisive is seeing both forms in one document: if one candidate is
/// a suffix of another, separated by a wholly-alphabetic prefix, the longer one
/// is the glue artifact. When only the glued form exists there is no way to
/// tell, and it is kept — a wrong guess here would silently discard a real
/// address, which the ledger makes unrecoverable.
fn drop_glued_variants(candidates: BTreeSet<String>) -> BTreeSet<String> {
    let all: Vec<&String> = candidates.iter().collect();
    let mut drop: BTreeSet<String> = BTreeSet::new();
    for long in &all {
        let Some(at) = long.find('@') else { continue };
        let (local, domain) = long.split_at(at);
        for short in &all {
            if *short == *long {
                continue;
            }
            let Some(sat) = short.find('@') else { continue };
            let (slocal, sdomain) = short.split_at(sat);
            if domain != sdomain || slocal.len() >= local.len() {
                continue;
            }
            // The short form must be a true suffix of the long local part.
            let Some(prefix) = local.strip_suffix(slocal) else {
                continue;
            };
            if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_alphabetic()) {
                drop.insert((*long).clone());
            }
        }
    }
    candidates
        .into_iter()
        .filter(|c| !drop.contains(c))
        .collect()
}

/// Why a candidate address-shaped string was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// An asset filename such as `sprite@2x.png`.
    AssetFilename,
    /// A reserved example/test domain.
    Placeholder,
    /// A Fediverse handle, not a mailbox.
    FederationHandle,
    /// The local part contains URL or punctuation characters.
    UrlFragment,
    /// The domain has no dot or the local part is empty.
    Malformed,
}

/// Characters that cannot appear unquoted in an RFC 5322 local part.
///
/// Catches URL-shaped matches such as `//medium.com/@victor.ronin`, where the
/// regex sees `@` and takes everything before it as the local part. A slash,
/// colon or backslash there means the "address" is really a URL path, which
/// rendering into Markdown makes indistinguishable from a mailbox at a glance.
const INVALID_LOCAL_CHARS: &[char] = &['/', ':', '\\', '<', '>', '"', '(', ')', ',', ';'];

/// Classify a candidate, returning the reason it is not an address.
///
/// Public so tests and callers can assert *why* something was dropped rather
/// than only that it was.
pub fn reject_reason(address: &str) -> Option<Rejected> {
    let Some((local, domain)) = address.rsplit_once('@') else {
        return Some(Rejected::Malformed);
    };
    if local.is_empty() || domain.is_empty() {
        return Some(Rejected::Malformed);
    }
    if local.contains(':') && local.contains('/') || local.starts_with("//") {
        return Some(Rejected::UrlFragment);
    }
    if local.chars().any(|c| INVALID_LOCAL_CHARS.contains(&c)) {
        return Some(Rejected::UrlFragment);
    }
    if !domain.contains('.') {
        return Some(Rejected::Malformed);
    }

    let domain_lower = domain.to_lowercase();
    let tld = domain_lower.rsplit('.').next().unwrap_or_default();
    if ASSET_EXTENSIONS.contains(&tld) {
        return Some(Rejected::AssetFilename);
    }
    if PLACEHOLDER_DOMAINS
        .iter()
        .any(|p| domain_lower == *p || domain_lower.ends_with(&format!(".{p}")))
    {
        return Some(Rejected::Placeholder);
    }
    if FEDERATION_HOSTS.iter().any(|h| domain_lower == *h) {
        return Some(Rejected::FederationHandle);
    }
    None
}

/// Extract addresses from a page's visible text (rendered Markdown).
///
/// Sorted and deduplicated. Rejected candidates are dropped, not returned —
/// callers that need the rejections should call [`reject_reason`] directly.
pub fn extract_from_text(text: &str) -> Vec<Found> {
    let mut raw: BTreeSet<String> = BTreeSet::new();
    for m in address_regex().find_iter(text) {
        let candidate = m.as_str().to_lowercase();
        if reject_reason(&candidate).is_none() {
            raw.insert(candidate);
        }
    }
    drop_glued_variants(raw)
        .into_iter()
        .map(Found::new)
        .collect()
}

/// Extract addresses from `mailto:` targets in raw HTML.
///
/// This is the higher-confidence source: a `mailto:` was authored as a contact
/// link, so it is not an accidental string match. Only the address is kept;
/// `?subject=` parameters are discarded.
pub fn extract_from_html(html: &str) -> Vec<Found> {
    let mut raw: BTreeSet<String> = BTreeSet::new();
    for cap in mailto_regex().captures_iter(html) {
        let Some(raw_match) = cap.get(1) else {
            continue;
        };
        let decoded = raw_match.as_str().replace("%40", "@").replace("&amp;", "");
        // `mailto:` targets may carry a trailing comma or sentence punctuation.
        let trimmed = decoded.trim_end_matches(|c: char| {
            c == ',' || c == ';' || c == '.' || c == '\\' || c == '"' || c == '\''
        });
        let candidate = trimmed.to_lowercase();
        if candidate.contains('@') && reject_reason(&candidate).is_none() {
            raw.insert(candidate);
        }
    }
    drop_glued_variants(raw)
        .into_iter()
        .map(Found::new)
        .collect()
}

/// Extract from both sources and merge, `mailto:` targets winning.
///
/// Deduplicated across sources, so an address that appears both as a link and
/// in body text is returned once.
pub fn extract_all(html: Option<&str>, text: &str) -> Vec<Found> {
    let mut out: BTreeSet<Found> = BTreeSet::new();
    for f in extract_from_html(html.unwrap_or_default()) {
        out.insert(f);
    }
    for f in extract_from_text(text) {
        out.insert(f);
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addrs(found: Vec<Found>) -> Vec<String> {
        found.into_iter().map(|f| f.address).collect()
    }

    #[test]
    fn finds_a_plain_address_in_text() {
        let got = addrs(extract_from_text("Contact us at build@dirmacs.com today."));
        assert_eq!(got, vec!["build@dirmacs.com"]);
    }

    #[test]
    fn finds_address_in_html_markdown_link() {
        // The shape extraction actually produces: a Markdown link whose text
        // is the address itself.
        let got = addrs(extract_from_text(
            "Send me an email at [hi@corrode.dev](mailto:hi@corrode.dev).",
        ));
        assert_eq!(got, vec!["hi@corrode.dev"]);
    }

    #[test]
    fn drops_glued_label_when_the_real_address_is_present() {
        // Observed on dirmacs.com: link text "Email" abutted its mailto target,
        // and the clean form appears separately in the body. Seeing both makes
        // the artifact identifiable without guessing at the local part.
        let got = addrs(extract_from_text(
            "Emailbuild@dirmacs.com ... or write to build@dirmacs.com",
        ));
        assert_eq!(got, vec!["build@dirmacs.com"]);
    }

    #[test]
    fn keeps_an_ambiguous_address_when_it_is_the_only_one() {
        // With no clean counterpart there is no way to tell `emailbuild` from
        // `Email` + `build`. Guessing would silently discard a real address,
        // which the ledger makes unrecoverable — so it is kept.
        let got = addrs(extract_from_text("Emailbuild@dirmacs.com"));
        assert_eq!(got, vec!["emailbuild@dirmacs.com"]);
    }

    #[test]
    fn keeps_a_capitalised_local_part() {
        // `JohnSmith@` must survive: it is not a suffix-glue of a shorter
        // address in the same document.
        let got = addrs(extract_from_text("write to JohnSmith@acme.com"));
        assert_eq!(got, vec!["johnsmith@acme.com"]);
    }

    #[test]
    fn rejects_asset_filenames() {
        let got = addrs(extract_from_text(
            "images: homepage@1440.webp, toys@760.webp, sprite@2x.png",
        ));
        assert!(
            got.is_empty(),
            "asset filenames must not be addresses: {got:?}"
        );
        assert_eq!(
            reject_reason("homepage@1440.webp"),
            Some(Rejected::AssetFilename)
        );
    }

    #[test]
    fn rejects_placeholder_domains() {
        let got = addrs(extract_from_text("alice@example.com and bob@test.invalid"));
        assert!(got.is_empty(), "placeholders must be dropped: {got:?}");
        assert_eq!(
            reject_reason("alice@example.com"),
            Some(Rejected::Placeholder)
        );
    }

    #[test]
    fn rejects_federation_handles() {
        assert_eq!(
            reject_reason("mainmatter@fosstodon.org"),
            Some(Rejected::FederationHandle)
        );
        let got = addrs(extract_from_text("find us at mainmatter@fosstodon.org"));
        assert!(got.is_empty());
    }

    #[test]
    fn rejects_url_fragments() {
        // Observed on corrode.dev: `//medium.com/@victor.ronin` renders as a
        // link and the regex reads everything before `@` as a local part.
        assert_eq!(
            reject_reason("//medium.com/@victor.ronin"),
            Some(Rejected::UrlFragment)
        );
        let got = addrs(extract_from_text("see //medium.com/@victor.ronin for more"));
        assert!(
            got.is_empty(),
            "URL fragments must not be addresses: {got:?}"
        );
    }

    #[test]
    fn mailto_trailing_backslash_is_trimmed() {
        // Observed on oxide.computer: a rendered link left a trailing escape.
        let html = r#"<a href="mailto:privacy@oxidecomputer.com\">Privacy</a>"#;
        let got = addrs(extract_from_html(html));
        assert_eq!(got, vec!["privacy@oxidecomputer.com"]);
    }

    #[test]
    fn keeps_real_addresses_that_resemble_rejects() {
        // Guard against over-filtering: a real company domain must survive.
        for addr in [
            "sales@oxide.computer",
            "privacy@oxidecomputer.com",
            "info@mainmatter.com",
            "rain@oxide.computer",
        ] {
            assert_eq!(reject_reason(addr), None, "{addr} must be accepted");
        }
    }

    #[test]
    fn mailto_beats_text_and_dedupes() {
        let html = r#"<a href="mailto:build@dirmacs.com">Email</a>"#;
        let got = addrs(extract_all(Some(html), "or write to build@dirmacs.com"));
        assert_eq!(got, vec!["build@dirmacs.com"]);
    }

    #[test]
    fn mailto_with_query_parameters_yields_bare_address() {
        let html = r#"<a href="mailto:sales@acme.com?subject=Hello%20there">Contact</a>"#;
        let got = addrs(extract_from_html(html));
        assert_eq!(got, vec!["sales@acme.com"]);
    }

    #[test]
    fn mailto_percent_encoded_at_is_decoded() {
        let html = r#"<a href="mailto:hello%40acme.com">Mail</a>"#;
        let got = addrs(extract_from_html(html));
        assert_eq!(got, vec!["hello@acme.com"]);
    }

    #[test]
    fn extraction_does_not_start_mid_token() {
        // `foo_bar@` style junk in a URL query must not yield a partial match
        // that looks valid.
        let got = addrs(extract_from_text("https://x.test/?u=notan@ddress"));
        assert!(
            !got.iter().any(|a| a == "notan@ddress"),
            "must not produce a dotless-domain address: {got:?}"
        );
    }

    #[test]
    fn duplicates_across_a_page_collapse() {
        let got = addrs(extract_from_text(
            "build@dirmacs.com ... BUILD@dirmacs.com ... build@dirmacs.com",
        ));
        assert_eq!(got, vec!["build@dirmacs.com"]);
    }
}
