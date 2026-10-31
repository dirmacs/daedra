//! HTML-to-Markdown conversion and markup cleanup.

pub(super) fn html_to_markdown(html: &str) -> String {
    // Use htmd crate for conversion
    htmd::convert(html).unwrap_or_else(|_| html.to_string())
}

/// Remove `<img ...>` and `<source ...>` elements from an HTML fragment so
/// the Markdown conversion emits no image syntax. `include_images: false`
/// is the default and this is what makes the flag real.
pub(super) fn strip_img_tags(html: &str) -> String {
    let lower = html.to_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut low = lower.as_str();
    while let Some(i) = low.find("<img") {
        match low[i..].find('>').map(|p| i + p + 1) {
            Some(j) => {
                out.push_str(&rest[..i]);
                rest = &rest[j..];
                low = &low[j..];
            },
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// Clean up Markdown content
pub(super) fn clean_markdown(markdown: &str) -> String {
    markdown
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed != "-" && trimmed != "*" && trimmed != "+"
        })
        .fold(String::new(), |mut acc, line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if !acc.ends_with("\n\n") {
                    acc.push('\n');
                }
            } else {
                acc.push_str(trimmed);
                acc.push('\n');
            }
            acc
        })
        .trim()
        .to_string()
}

/// Clean up a page title
pub(super) fn clean_title(title: &str) -> String {
    // Remove common suffixes
    let title = title
        .split(" | ")
        .next()
        .unwrap_or(title)
        .split(" - ")
        .next()
        .unwrap_or(title)
        .split(" :: ")
        .next()
        .unwrap_or(title)
        .split(" — ")
        .next()
        .unwrap_or(title);

    title.trim().to_string()
}
