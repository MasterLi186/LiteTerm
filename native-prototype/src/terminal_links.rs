const MAX_LOGICAL_LINE_CHARS: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Url(String),
    LocalPath {
        path: String,
        line: Option<u32>,
        column: Option<u32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkCell {
    pub ch: char,
    pub start_col: usize,
    pub width: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSpan {
    pub start_col: usize,
    pub end_col: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalLink {
    pub target: LinkTarget,
    pub span: GridSpan,
}

pub fn link_at(
    cells: &[LinkCell],
    point_col: usize,
    explicit_osc8: Option<&str>,
    allow_local_paths: bool,
    allow_relative_paths: bool,
) -> Option<TerminalLink> {
    let cells = bounded_cells(cells);
    let point_index = cells.iter().position(|cell| {
        point_col >= cell.start_col && point_col < cell.start_col.saturating_add(cell.width.max(1))
    })?;
    if let Some(uri) = explicit_osc8 {
        if let Some(url) = normalize_http_url(uri) {
            return Some(TerminalLink {
                target: LinkTarget::Url(url),
                span: GridSpan {
                    start_col: cells[point_index].start_col,
                    end_col: cells[point_index]
                        .start_col
                        .saturating_add(cells[point_index].width.max(1)),
                },
            });
        }
        if allow_local_paths {
            if let Some(target) = normalize_file_url(uri) {
                return Some(TerminalLink {
                    target,
                    span: GridSpan {
                        start_col: cells[point_index].start_col,
                        end_col: cells[point_index]
                            .start_col
                            .saturating_add(cells[point_index].width.max(1)),
                    },
                });
            }
        }
    }

    let (token_start, token_end) = token_bounds(cells, point_index);
    let token_cells = &cells[token_start..token_end];
    let token = token_cells.iter().map(|cell| cell.ch).collect::<String>();
    if let Some((url, url_start, url_end)) =
        http_url_at(&token, point_index.saturating_sub(token_start))
    {
        let start = cells.get(token_start + url_start)?.start_col;
        let last = cells.get(token_start + url_end.checked_sub(1)?)?;
        return Some(TerminalLink {
            target: LinkTarget::Url(url),
            span: GridSpan {
                start_col: start,
                end_col: last.start_col.saturating_add(last.width.max(1)),
            },
        });
    }
    let (trimmed, trim_left, trim_right) = trim_token(&token);
    if trimmed.is_empty() {
        return None;
    }
    let target = normalize_http_url(trimmed)
        .map(LinkTarget::Url)
        .or_else(|| {
            allow_local_paths
                .then(|| normalize_file_url(trimmed))
                .flatten()
        })
        .or_else(|| {
            allow_local_paths
                .then(|| normalize_local_path(trimmed, allow_relative_paths))
                .flatten()
        })?;
    let start_index = token_start.saturating_add(trim_left);
    let end_index = token_end.saturating_sub(trim_right);
    let start = cells.get(start_index)?.start_col;
    let last = cells.get(end_index.checked_sub(1)?)?;
    let end = last.start_col.saturating_add(last.width.max(1));
    Some(TerminalLink {
        target,
        span: GridSpan {
            start_col: start,
            end_col: end,
        },
    })
}

fn http_url_at(token: &str, point_char: usize) -> Option<(String, usize, usize)> {
    let chars = token.chars().collect::<Vec<_>>();
    for (start_byte, _) in token.char_indices() {
        let start_char = token[..start_byte].chars().count();
        let suffix = &token[start_byte..];
        if !(suffix.starts_with("http://") || suffix.starts_with("https://")) {
            continue;
        }
        let (candidate, _, trim_right) = trim_token(suffix);
        let end_char = chars.len().saturating_sub(trim_right);
        if point_char < start_char || point_char >= end_char {
            continue;
        }
        if let Some(url) = normalize_http_url(candidate) {
            return Some((url, start_char, end_char));
        }
    }
    None
}

fn bounded_cells(cells: &[LinkCell]) -> &[LinkCell] {
    &cells[..cells.len().min(MAX_LOGICAL_LINE_CHARS)]
}

fn token_bounds(cells: &[LinkCell], point_index: usize) -> (usize, usize) {
    let mut start = point_index;
    while start > 0 && !is_token_separator(cells[start - 1].ch) {
        start -= 1;
    }
    let mut end = point_index + 1;
    while end < cells.len() && !is_token_separator(cells[end].ch) {
        end += 1;
    }
    (start, end)
}

fn is_token_separator(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '>')
}

fn trim_token(token: &str) -> (&str, usize, usize) {
    let chars = token.chars().collect::<Vec<_>>();
    let mut start = 0;
    let mut end = chars.len();
    while start < end && matches!(chars[start], '(' | '[' | '{') {
        start += 1;
    }
    while end > start && is_trailing_punctuation(chars[end - 1], &chars[start..end]) {
        end -= 1;
    }
    let start_byte = chars[..start].iter().map(|ch| ch.len_utf8()).sum::<usize>();
    let end_byte = chars[..end].iter().map(|ch| ch.len_utf8()).sum::<usize>();
    (&token[start_byte..end_byte], start, chars.len() - end)
}

fn is_trailing_punctuation(ch: char, token: &[char]) -> bool {
    match ch {
        '.' | ',' | ';' | '!' | '?' => true,
        ')' => {
            token.iter().filter(|item| **item == '(').count()
                < token.iter().filter(|item| **item == ')').count()
        }
        ']' => {
            token.iter().filter(|item| **item == '[').count()
                < token.iter().filter(|item| **item == ']').count()
        }
        '}' => {
            token.iter().filter(|item| **item == '{').count()
                < token.iter().filter(|item| **item == '}').count()
        }
        _ => false,
    }
}

fn normalize_http_url(raw: &str) -> Option<String> {
    if raw.chars().any(char::is_control) || raw.chars().any(char::is_whitespace) {
        return None;
    }
    let rest = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))?;
    if rest.is_empty() {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.starts_with(':') {
        return None;
    }
    if authority.starts_with('[') && !authority.contains(']') {
        return None;
    }
    Some(raw.to_owned())
}

fn normalize_file_url(raw: &str) -> Option<LinkTarget> {
    let rest = raw.strip_prefix("file://")?;
    let encoded_path = if rest.starts_with('/') {
        rest
    } else if let Some(path) = rest.strip_prefix("localhost/") {
        &raw[raw.len() - path.len() - 1..]
    } else {
        return None;
    };
    if encoded_path.contains(['?', '#']) {
        return None;
    }
    let path = percent_decode_utf8(encoded_path)?;
    normalize_local_path(&path, false)
}

fn percent_decode_utf8(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        let high = hex_value(*bytes.get(index + 1)?)?;
        let low = hex_value(*bytes.get(index + 2)?)?;
        decoded.push((high << 4) | low);
        index += 3;
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if decoded.chars().any(char::is_control) {
        return None;
    }
    Some(decoded)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn normalize_local_path(raw: &str, allow_relative: bool) -> Option<LinkTarget> {
    if raw.starts_with("//") || raw.chars().any(char::is_control) {
        return None;
    }
    let (path, line, column) = split_diagnostic_position(raw);
    let allowed = path.starts_with('/')
        || path.starts_with("~/")
        || (allow_relative && (path.starts_with("./") || path.starts_with("../")));
    if !allowed || matches!(path, "/" | "~/" | "./" | "../") {
        return None;
    }
    Some(LinkTarget::LocalPath {
        path: path.to_owned(),
        line,
        column,
    })
}

fn split_diagnostic_position(raw: &str) -> (&str, Option<u32>, Option<u32>) {
    let Some((before_last, last)) = raw.rsplit_once(':') else {
        return (raw, None, None);
    };
    let Ok(last_number) = last.parse::<u32>() else {
        return (raw, None, None);
    };
    if let Some((path, line)) = before_last.rsplit_once(':') {
        if let Ok(line_number) = line.parse::<u32>() {
            return (path, Some(line_number), Some(last_number));
        }
    }
    (before_last, Some(last_number), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(text: &str) -> Vec<LinkCell> {
        let mut column = 0;
        text.chars()
            .map(|ch| {
                let width = if ch == '终' { 2 } else { 1 };
                let cell = LinkCell {
                    ch,
                    start_col: column,
                    width,
                };
                column += width;
                cell
            })
            .collect()
    }

    #[test]
    fn finds_http_url_and_trims_sentence_punctuation() {
        let line = cells("查看 https://example.com/a?q=1#x).");
        let link = link_at(&line, 10, None, true, false).unwrap();
        assert_eq!(
            link.target,
            LinkTarget::Url("https://example.com/a?q=1#x".into())
        );
    }

    #[test]
    fn accepts_ipv6_authority_and_rejects_unsafe_schemes() {
        let ipv6 = cells("http://[::1]:8080/a");
        assert!(matches!(
            link_at(&ipv6, 2, None, false, false).unwrap().target,
            LinkTarget::Url(_)
        ));
        for unsafe_text in ["javascript:alert(1)", "data:text/plain,x", "ftp://host/x"] {
            assert!(link_at(&cells(unsafe_text), 2, None, true, true).is_none());
        }
    }

    #[test]
    fn path_position_is_separated_from_open_target() {
        let line = cells("/tmp/main.rs:12:4");
        assert_eq!(
            link_at(&line, 2, None, true, false).unwrap().target,
            LinkTarget::LocalPath {
                path: "/tmp/main.rs".into(),
                line: Some(12),
                column: Some(4),
            }
        );
    }

    #[test]
    fn relative_paths_require_explicit_permission() {
        let line = cells("../src/main.rs:9");
        assert!(link_at(&line, 2, None, true, false).is_none());
        assert!(matches!(
            link_at(&line, 2, None, true, true).unwrap().target,
            LinkTarget::LocalPath { .. }
        ));
    }

    #[test]
    fn remote_callers_can_disable_all_path_targets() {
        assert!(link_at(&cells("/etc/passwd"), 3, None, false, false).is_none());
        assert!(matches!(
            link_at(&cells("https://example.com"), 3, None, false, false)
                .unwrap()
                .target,
            LinkTarget::Url(_)
        ));
    }

    #[test]
    fn wide_character_column_mapping_is_preserved() {
        let line = cells("终https://x.test");
        let link = link_at(&line, 4, None, false, false).unwrap();
        assert_eq!(link.span.start_col, 2);
        assert_eq!(link.span.end_col, 16);
    }

    #[test]
    fn osc8_candidate_has_priority_but_still_uses_scheme_allowlist() {
        let line = cells("click");
        assert_eq!(
            link_at(&line, 2, Some("https://osc.example/a"), false, false)
                .unwrap()
                .target,
            LinkTarget::Url("https://osc.example/a".into())
        );
        assert_eq!(
            link_at(&line, 2, Some("file:///tmp/x"), true, false)
                .unwrap()
                .target,
            LinkTarget::LocalPath {
                path: "/tmp/x".into(),
                line: None,
                column: None,
            }
        );
        assert!(link_at(&line, 2, Some("file:///tmp/x"), false, false).is_none());
    }

    #[test]
    fn file_urls_are_local_only_and_decode_safe_absolute_paths() {
        let line = cells("file:///tmp/a%20b.txt");
        assert_eq!(
            link_at(&line, 8, None, true, false).unwrap().target,
            LinkTarget::LocalPath {
                path: "/tmp/a b.txt".into(),
                line: None,
                column: None,
            }
        );
        let localhost = cells("file://localhost/tmp/x");
        assert!(matches!(
            link_at(&localhost, 8, None, true, false).unwrap().target,
            LinkTarget::LocalPath { path, .. } if path == "/tmp/x"
        ));
        for rejected in [
            "file://remote/tmp/x",
            "file:///tmp/x?query",
            "file:///tmp/x#fragment",
            "file:///tmp/%00x",
            "file:///tmp/%zz",
        ] {
            assert!(link_at(&cells(rejected), 8, None, true, false).is_none());
        }
    }

    #[test]
    fn logical_line_scan_is_bounded() {
        let mut line = cells(&"x".repeat(MAX_LOGICAL_LINE_CHARS));
        let url_start = line.len();
        line.extend(
            cells("https://outside.example")
                .into_iter()
                .map(|mut cell| {
                    cell.start_col += url_start;
                    cell
                }),
        );
        assert!(link_at(&line, url_start + 2, None, false, false).is_none());
    }

    #[test]
    fn diagnostic_suffix_does_not_turn_path_roots_into_openable_targets() {
        for root in ["/:12", "~/:12", "./:12", "../:12:3"] {
            assert!(
                link_at(&cells(root), 1, None, true, true).is_none(),
                "{root} must remain a rejected root path"
            );
        }
    }
}
