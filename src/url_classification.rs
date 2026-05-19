use crate::types::ContentType;

struct SubstringRule {
    content_type: ContentType,
    patterns: &'static [&'static str],
}

const SUBSTRING_RULES: &[SubstringRule] = &[
    SubstringRule {
        content_type: ContentType::Documentation,
        patterns: &[
            "docs.",
            "/docs/",
            "/documentation/",
            "readthedocs",
            "javadoc",
            "/api/",
        ],
    },
    SubstringRule {
        content_type: ContentType::Documentation,
        patterns: &[
            "github.com",
            "gitlab.com",
            "stackoverflow.com",
            "stackexchange.com",
            "bitbucket.org",
        ],
    },
    SubstringRule {
        content_type: ContentType::Social,
        patterns: &[
            "twitter.com",
            "x.com",
            "facebook.com",
            "linkedin.com",
            "instagram.com",
            "tiktok.com",
        ],
    },
    SubstringRule {
        content_type: ContentType::Forum,
        patterns: &["reddit.com", "forum", "discourse", "community."],
    },
    SubstringRule {
        content_type: ContentType::Video,
        patterns: &["youtube.com", "youtu.be", "vimeo.com", "twitch.tv"],
    },
    SubstringRule {
        content_type: ContentType::Shopping,
        patterns: &["amazon.", "ebay.", "shop.", "/shop/", "store."],
    },
    SubstringRule {
        content_type: ContentType::Article,
        patterns: &["news.", "/news/", "bbc.", "cnn.", "nytimes.", "reuters."],
    },
];

/// Classify a search result URL into a [`ContentType`] using substring rules.
pub fn classify_search_url(url: &str) -> ContentType {
    let lower = url.to_lowercase();
    for rule in SUBSTRING_RULES {
        if rule.patterns.iter().any(|p| lower.contains(p)) {
            return rule.content_type;
        }
    }
    ContentType::Article
}
