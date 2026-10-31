//! Query analysis: language detection and topic tagging.

use super::content::{ContentType, SearchResult};

struct LangRange {
    lang: &'static str,
    ranges: &'static [(char, char)],
}

const LANG_RANGES: &[LangRange] = &[
    LangRange {
        lang: "zh",
        ranges: &[('\u{4e00}', '\u{9fff}')],
    },
    LangRange {
        lang: "ja",
        ranges: &[('\u{3040}', '\u{30ff}')],
    },
    LangRange {
        lang: "ko",
        ranges: &[('\u{ac00}', '\u{d7af}')],
    },
    LangRange {
        lang: "ru",
        ranges: &[('\u{0400}', '\u{04ff}')],
    },
    LangRange {
        lang: "ar",
        ranges: &[('\u{0600}', '\u{06ff}')],
    },
];

/// Detect language of a query using simple heuristics
pub(super) fn detect_language(query: &str) -> String {
    for range in LANG_RANGES {
        if query
            .chars()
            .any(|c| range.ranges.iter().any(|&(s, e)| c >= s && c <= e))
        {
            return range.lang.to_string();
        }
    }
    "en".to_string()
}

struct TopicRule {
    topic: &'static str,
    url_patterns: &'static [&'static str],
    title_patterns: &'static [&'static str],
    content_type: Option<ContentType>,
}

const TOPIC_RULES: &[TopicRule] = &[
    TopicRule {
        topic: "technology",
        url_patterns: &["github.com", "stackoverflow.com", "gitlab.com"],
        title_patterns: &["programming", "code"],
        content_type: None,
    },
    TopicRule {
        topic: "documentation",
        url_patterns: &["docs.", "/docs/", "/documentation/"],
        title_patterns: &["documentation", "api reference"],
        content_type: None,
    },
    TopicRule {
        topic: "news",
        url_patterns: &["news.", "/news/"],
        title_patterns: &[],
        content_type: Some(ContentType::Article),
    },
    TopicRule {
        topic: "academic",
        url_patterns: &[".edu", "arxiv.org", "scholar.google"],
        title_patterns: &["research", "study"],
        content_type: None,
    },
];

fn matches_topic_rule(result: &SearchResult, rule: &TopicRule) -> bool {
    let lower_url = result.url.to_lowercase();
    let lower_title = result.title.to_lowercase();
    let url_match = rule.url_patterns.iter().any(|p| lower_url.contains(p));
    let title_match = rule.title_patterns.iter().any(|p| lower_title.contains(p));
    let type_match = rule
        .content_type
        .is_none_or(|ct| result.metadata.content_type == ct);
    url_match || title_match || type_match
}

/// Detect topics from search results
pub(super) fn detect_topics(results: &[SearchResult]) -> Vec<String> {
    let mut topics = std::collections::HashSet::new();
    for result in results {
        for rule in TOPIC_RULES {
            if matches_topic_rule(result, rule) {
                topics.insert(rule.topic.to_string());
            }
        }
    }
    topics.into_iter().collect()
}
