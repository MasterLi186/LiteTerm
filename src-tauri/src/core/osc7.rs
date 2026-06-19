//! OSC 7 当前目录上报序列解析。
//!
//! shell 通过 `ESC ] 7 ; file://<host>/<path> (BEL | ESC \)` 上报 cwd。
//! 终端 reader 线程把所有输出字节喂给本解析器，解析出最近一次的远端路径。

/// 累积缓冲上限，防止异常流导致内存增长。
const MAX_PENDING: usize = 8192;

/// 增量 OSC7 解析器：跨多次 feed 处理可能被切分的序列。
pub struct Osc7Parser {
    pending: Vec<u8>,
}

impl Osc7Parser {
    pub fn new() -> Self {
        Self { pending: Vec::new() }
    }

    /// 喂入一段终端输出字节。返回本次累积流中最新一条完整 OSC7 解析出的路径
    /// （若有多条取最后一条）；没有完整序列时返回 None。
    pub fn feed(&mut self, data: &[u8]) -> Option<String> {
        self.pending.extend_from_slice(data);
        if self.pending.len() > MAX_PENDING {
            let cut = self.pending.len() - MAX_PENDING;
            self.pending.drain(..cut);
        }

        let prefix = [0x1b, b']', b'7', b';']; // ESC ] 7 ;
        let mut result = None;
        loop {
            let Some(s) = find_subseq(&self.pending, &prefix) else {
                // 没有起始标记：只保留末尾最多 3 字节（可能是被切断的 ESC ] 7 前缀）
                if self.pending.len() > 3 {
                    let cut = self.pending.len() - 3;
                    self.pending.drain(..cut);
                }
                break;
            };
            let payload_start = s + prefix.len();
            // 从 payload 起找终止符：BEL(0x07) 或 ST(ESC '\')
            let mut term: Option<(usize, usize)> = None; // (index, len)
            let mut i = payload_start;
            while i < self.pending.len() {
                if self.pending[i] == 0x07 {
                    term = Some((i, 1));
                    break;
                }
                if self.pending[i] == 0x1b
                    && i + 1 < self.pending.len()
                    && self.pending[i + 1] == 0x5c
                {
                    term = Some((i, 2));
                    break;
                }
                i += 1;
            }
            let Some((t, tlen)) = term else {
                // 序列未结束：丢掉起始标记之前的内容，等下次 feed 补齐
                self.pending.drain(..s);
                break;
            };
            let payload = self.pending[payload_start..t].to_vec();
            self.pending.drain(..t + tlen);
            if let Some(path) = parse_file_url(&payload) {
                result = Some(path);
            }
        }
        result
    }
}

/// 在 haystack 中查找子序列 needle 的起始下标。
fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

/// 从 OSC7 payload 解析出路径。payload 形如 `file://host/path`，也兼容直接是路径。
/// 对 `%XX` 做 URL 解码。接受任意 host（含 shell 原生 OSC7，如 fish），与主流终端
/// 一致；拖拽上传的目标目录会在前端浮窗显示，作为可见的安全提示。
fn parse_file_url(payload: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(payload);
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let path_part = if let Some(rest) = s.strip_prefix("file://") {
        // rest = host/path —— 从 host 之后的第一个 '/' 开始才是路径
        match rest.find('/') {
            Some(idx) => &rest[idx..],
            None => return None,
        }
    } else if s.starts_with('/') {
        s
    } else {
        return None;
    };
    let decoded = url_decode(path_part);
    if decoded.starts_with('/') {
        Some(decoded)
    } else {
        None
    }
}

/// 最小化 URL 解码：把 %XX 还原为字节，其余原样。
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complete_bel() {
        let mut p = Osc7Parser::new();
        let seq = b"\x1b]7;file://host/home/bmc\x07";
        assert_eq!(p.feed(seq), Some("/home/bmc".to_string()));
    }

    #[test]
    fn test_complete_st() {
        let mut p = Osc7Parser::new();
        let seq = b"\x1b]7;file://host/var/log\x1b\\";
        assert_eq!(p.feed(seq), Some("/var/log".to_string()));
    }

    #[test]
    fn test_split_across_feeds() {
        let mut p = Osc7Parser::new();
        assert_eq!(p.feed(b"\x1b]7;file://h/ho"), None);
        assert_eq!(p.feed(b"me/lfl/work\x07"), Some("/home/lfl/work".to_string()));
    }

    #[test]
    fn test_url_decode() {
        let mut p = Osc7Parser::new();
        let seq = b"\x1b]7;file://h/home/a%20b/c\x07";
        assert_eq!(p.feed(seq), Some("/home/a b/c".to_string()));
    }

    #[test]
    fn test_surrounding_garbage() {
        let mut p = Osc7Parser::new();
        let seq = b"some prompt text\x1b]7;file://h/data/bmc\x07lfl@host:~$ ";
        assert_eq!(p.feed(seq), Some("/data/bmc".to_string()));
    }

    #[test]
    fn test_latest_of_multiple() {
        let mut p = Osc7Parser::new();
        let seq = b"\x1b]7;file://h/a\x07\x1b]7;file://h/b\x07";
        assert_eq!(p.feed(seq), Some("/b".to_string()));
    }

    #[test]
    fn test_no_osc7() {
        let mut p = Osc7Parser::new();
        assert_eq!(p.feed(b"just normal terminal output\r\n"), None);
    }

    #[test]
    fn test_oversized_does_not_grow_unbounded() {
        let mut p = Osc7Parser::new();
        // 一大段没有完整 OSC7 的数据
        let big = vec![b'x'; 20000];
        assert_eq!(p.feed(&big), None);
        // pending 不应超过上限
        assert!(p.pending.len() <= MAX_PENDING);
    }
}
