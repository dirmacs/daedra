//! Content extraction for non-HTML payloads: PDF, office documents, and
//! inference-based classification when the declared content type is wrong.

use crate::types::{DaedraError, DaedraResult};

use super::client::FetchedContent;
use super::mime::{bytes_to_utf8_string, is_binary_mime, normalize_content_type};

pub(super) fn extract_pdf_content(bytes: &[u8]) -> DaedraResult<FetchedContent> {
    // pdf-inspector (MIT, zero deps) replaces pdf-extract: it emits
    // Markdown directly and reports which pages need OCR instead of
    // failing silently on scanned documents.
    match pdf_inspector::process_pdf_mem(bytes) {
        Ok(result) => {
            let markdown = result.markdown.unwrap_or_default();
            Ok(FetchedContent::Pdf(markdown))
        },
        Err(e) => Err(DaedraError::ExtractionError(e.to_string())),
    }
}

pub(super) fn classify_inferred_mime(mime: &str, bytes: &[u8]) -> Option<FetchedContent> {
    match mime {
        "application/pdf" => extract_pdf_content(bytes).ok(),
        "text/html" | "application/xhtml+xml" => {
            Some(FetchedContent::Html(bytes_to_utf8_string(bytes)))
        },
        // Office/ ebook documents convert to Markdown via anydoc.
        m if is_office_content_type(m) => extract_document_content(bytes, m).ok(),
        // Zip/octet-stream containers: anydoc sniffs the archive contents;
        // a plain zip falls back to Binary.
        m if is_container_mime(m) => match extract_document_content(bytes, m) {
            Ok(doc) => Some(doc),
            Err(_) => Some(FetchedContent::Binary {
                mime: m.to_string(),
                size: bytes.len(),
            }),
        },
        // SVG and other textual XML are content, not opaque binaries: hand
        // the source to the caller instead of "unsupported content type".
        m if is_textual_content_type(m) => Some(FetchedContent::Text {
            mime: m.to_string(),
            body: bytes_to_utf8_string(bytes),
        }),
        m if is_binary_mime(m) => Some(FetchedContent::Binary {
            mime: m.to_string(),
            size: bytes.len(),
        }),
        m if m.starts_with("text/") => Some(FetchedContent::Html(bytes_to_utf8_string(bytes))),
        _ => None,
    }
}

pub(super) fn classify_by_inference(kind: &infer::Type, bytes: &[u8]) -> Option<FetchedContent> {
    classify_inferred_mime(kind.mime_type(), bytes)
}

/// True for textual structured mimes (SVG, XML, JSON) that should reach the
/// caller verbatim instead of being rejected as binaries.
pub(super) fn is_textual_content_type(ct: &str) -> bool {
    ct.contains("svg") || ct.contains("xml") || ct.contains("json")
}

/// True for Office/ ebook content types that `anydoc` converts to
/// Markdown. Checked BEFORE the textual-XML rule: the docx/odt MIME strings
/// contain the substring "xml" but are zip containers, not text.
pub(super) fn is_office_content_type(ct: &str) -> bool {
    ct.contains("officedocument")
        || ct.contains("msword")
        || ct.contains("ms-powerpoint")
        || ct.contains("ms-excel")
        || ct.contains("epub")
        || ct.contains("opendocument")
        || ct == "application/rtf"
        || ct == "text/csv"
        || ct.contains("presentationml")
        || ct.contains("spreadsheetml")
}

/// Zip containers (docx, odt, epub, xlsx, pptx are all zips) sometimes
/// arrive typed only as `application/zip` or `application/octet-stream` —
/// many static file servers do this. anydoc detects the real format from
/// the archive contents; on failure it is an ordinary zip and stays binary.
pub(super) fn is_container_mime(ct: &str) -> bool {
    ct == "application/zip" || ct == "application/octet-stream"
}

/// Convert an Office/ ebook document to Markdown via `anydoc`. Format
/// detection runs on the bytes, so a wrong MIME header is survivable.
pub(super) fn extract_document_content(bytes: &[u8], ct: &str) -> DaedraResult<FetchedContent> {
    // CSV has no byte signature; anydoc requires the format named for it.
    let format = if normalize_content_type(ct) == "text/csv" {
        Some(anydoc::Format::Csv)
    } else {
        None
    };
    let markdown = anydoc::to_markdown_bytes(bytes, format)
        .map_err(|e| DaedraError::ExtractionError(format!("document convert: {e}")))?;
    Ok(FetchedContent::Document {
        mime: ct.to_string(),
        markdown,
    })
}

pub(super) fn classify_by_fallback(
    content_type: &str,
    bytes: &[u8],
) -> DaedraResult<FetchedContent> {
    let ct = normalize_content_type(content_type);
    if ct.contains("text/html") {
        return Ok(FetchedContent::Html(bytes_to_utf8_string(bytes)));
    }

    if is_office_content_type(&ct) {
        return extract_document_content(bytes, &ct);
    }

    if is_textual_content_type(&ct) {
        return Ok(FetchedContent::Text {
            mime: ct,
            body: bytes_to_utf8_string(bytes),
        });
    }

    if std::str::from_utf8(bytes).is_ok() {
        return Ok(FetchedContent::Html(bytes_to_utf8_string(bytes)));
    }

    Ok(FetchedContent::Binary {
        mime: if ct.is_empty() {
            "application/octet-stream".to_string()
        } else {
            ct
        },
        size: bytes.len(),
    })
}

pub(super) fn classify_fetched_content(
    content_type: &str,
    bytes: &[u8],
) -> DaedraResult<FetchedContent> {
    if let Some(kind) = infer::get(bytes) {
        if let Some(content) = classify_by_inference(&kind, bytes) {
            return Ok(content);
        }
        if kind.mime_type() == "application/pdf" {
            return extract_pdf_content(bytes);
        }
    }

    classify_by_fallback(content_type, bytes)
}
