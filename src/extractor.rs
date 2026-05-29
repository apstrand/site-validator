use html5ever::{
    local_name,
    tokenizer::{
        BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
    },
    tendril::StrTendril,
};
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Clone)]
pub struct ExtractedLink {
    pub url: String,
    pub line: Option<u32>,
    pub text: String,
    pub source: String,
}

// ---------------------------------------------------------------------------
// html5ever tokenizer sink
// ---------------------------------------------------------------------------

struct LinkSink {
    links: Vec<RawHref>,
    in_anchor: bool,
    current_href: Option<String>,
    current_line: u32,
    current_text: String,
}

struct RawHref {
    href: String,
    line: u32,
    text: String,
}

impl TokenSink for LinkSink {
    type Handle = ();

    fn process_token(&mut self, token: Token, line_number: u64) -> TokenSinkResult<()> {
        match token {
            Token::TagToken(Tag { kind, name, attrs, .. }) => {
                if name == local_name!("a") {
                    match kind {
                        TagKind::StartTag => {
                            if self.in_anchor {
                                self.flush();
                            }
                            self.in_anchor = true;
                            self.current_line = line_number as u32;
                            self.current_text.clear();
                            self.current_href = attrs
                                .into_iter()
                                .find(|a| a.name.local == local_name!("href"))
                                .map(|a| a.value.to_string());
                        }
                        TagKind::EndTag => {
                            self.flush();
                            self.in_anchor = false;
                        }
                    }
                }
            }
            Token::CharacterTokens(s) if self.in_anchor => {
                if self.current_text.len() < 200 {
                    self.current_text.push_str(&s);
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }

    fn end(&mut self) {
        self.flush();
    }
}

impl LinkSink {
    fn flush(&mut self) {
        if let Some(href) = self.current_href.take() {
            self.links.push(RawHref {
                href,
                line: self.current_line,
                text: self.current_text.trim().chars().take(120).collect(),
            });
        }
        self.current_text.clear();
    }
}

fn raw_hrefs(html: &[u8], _source: &str) -> Vec<RawHref> {
    let html_str = String::from_utf8_lossy(html);
    let sink = LinkSink {
        links: Vec::new(),
        in_anchor: false,
        current_href: None,
        current_line: 0,
        current_text: String::new(),
    };
    let mut tok = Tokenizer::new(sink, TokenizerOpts::default());
    let mut input = BufferQueue::default();
    input.push_back(StrTendril::from(html_str.as_ref()));
    let _ = tok.feed(&mut input);
    tok.end();
    tok.sink.links
}

// ---------------------------------------------------------------------------
// URL mode extraction
// ---------------------------------------------------------------------------

pub fn extract_links_url(html: &[u8], base_url: &str, source: &str) -> Vec<ExtractedLink> {
    let Ok(base) = Url::parse(base_url) else { return vec![] };
    raw_hrefs(html, source)
        .into_iter()
        .filter_map(|r| {
            let resolved = resolve_url_href(&r.href, &base)?;
            Some(ExtractedLink {
                url: resolved,
                line: Some(r.line),
                text: r.text,
                source: source.to_string(),
            })
        })
        .collect()
}

fn resolve_url_href(href: &str, base: &Url) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    let lower = href.to_lowercase();
    for skip in &["mailto:", "tel:", "javascript:", "data:", "ftp:"] {
        if lower.starts_with(skip) {
            return None;
        }
    }
    let mut resolved = base.join(href).ok()?;
    resolved.set_fragment(None);
    Some(resolved.to_string())
}

// ---------------------------------------------------------------------------
// Local mode extraction
// ---------------------------------------------------------------------------

pub fn extract_links_local(
    html: &[u8],
    file_path: &Path,
    root: &Path,
    source: &str,
    check_external: bool,
) -> Vec<ExtractedLink> {
    raw_hrefs(html, source)
        .into_iter()
        .filter_map(|r| {
            let resolved = resolve_local_href(&r.href, file_path, root)?;
            if !check_external && resolved.starts_with("http") {
                return None;
            }
            Some(ExtractedLink {
                url: resolved,
                line: Some(r.line),
                text: r.text,
                source: source.to_string(),
            })
        })
        .collect()
}

fn resolve_local_href(href: &str, file_path: &Path, root: &Path) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }

    // External URL
    if href.starts_with("http://") || href.starts_with("https://") {
        let clean = href.split('#').next()?.split('?').next()?.to_string();
        return Some(clean);
    }
    if href.starts_with("//") {
        let clean = href.split('#').next()?.split('?').next()?;
        return Some(format!("https:{}", clean));
    }

    // Skip non-web schemes
    if href.contains(':') && !href.starts_with('/') {
        return None;
    }

    // Strip fragment and query
    let clean = href.split('#').next()?.split('?').next()?;
    if clean.is_empty() {
        return None;
    }

    let resolved = if clean.starts_with('/') {
        root.join(clean.trim_start_matches('/'))
    } else {
        file_path.parent()?.join(clean)
    };

    Some(normalize_path(&resolved).to_string_lossy().into_owned())
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}
