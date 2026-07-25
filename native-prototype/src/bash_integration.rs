use base64::Engine;

use crate::smart_completion::CompletionSessionKey;

pub const MAX_OSC_FRAME: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    Prompt,
    HistoryPath(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkerBoundary {
    pub end_offset: usize,
    pub kind: MarkerKind,
}

pub struct MarkerDecoder {
    session: CompletionSessionKey,
    ground_escape: bool,
    in_osc: bool,
    osc_escape: bool,
    overflow: bool,
    frame: Vec<u8>,
}

impl MarkerDecoder {
    pub fn new(session: CompletionSessionKey) -> Self {
        Self {
            session,
            ground_escape: false,
            in_osc: false,
            osc_escape: false,
            overflow: false,
            frame: Vec::with_capacity(MAX_OSC_FRAME),
        }
    }

    pub fn scan(&mut self, chunk: &[u8]) -> Vec<MarkerBoundary> {
        let mut markers = Vec::new();

        for (offset, &byte) in chunk.iter().enumerate() {
            if !self.in_osc {
                if self.ground_escape {
                    self.ground_escape = byte == b'\x1b';
                    if byte == b']' {
                        self.in_osc = true;
                        self.ground_escape = false;
                        self.osc_escape = false;
                        self.overflow = false;
                        self.frame.clear();
                    }
                } else if byte == b'\x1b' {
                    self.ground_escape = true;
                }
                continue;
            }

            if self.osc_escape {
                self.osc_escape = false;
                if byte == b'\\' {
                    self.finish_frame(offset + 1, &mut markers);
                    continue;
                }

                self.push_frame_byte(b'\x1b');
                if byte == b'\x07' {
                    self.finish_frame(offset + 1, &mut markers);
                } else if byte == b'\x1b' {
                    self.osc_escape = true;
                } else {
                    self.push_frame_byte(byte);
                }
            } else if byte == b'\x07' {
                self.finish_frame(offset + 1, &mut markers);
            } else if byte == b'\x1b' {
                self.osc_escape = true;
            } else {
                self.push_frame_byte(byte);
            }
        }

        markers
    }

    fn push_frame_byte(&mut self, byte: u8) {
        if self.overflow {
            return;
        }
        if self.frame.len() == MAX_OSC_FRAME {
            self.overflow = true;
            self.frame.clear();
        } else {
            self.frame.push(byte);
        }
    }

    fn finish_frame(&mut self, end_offset: usize, markers: &mut Vec<MarkerBoundary>) {
        if !self.overflow {
            if let Some(kind) = parse_marker(&self.frame, &self.session) {
                markers.push(MarkerBoundary { end_offset, kind });
            }
        }

        self.ground_escape = false;
        self.in_osc = false;
        self.osc_escape = false;
        self.overflow = false;
        self.frame.clear();
    }
}

fn parse_marker(frame: &[u8], session: &CompletionSessionKey) -> Option<MarkerKind> {
    let text = std::str::from_utf8(frame).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }

    let fields = text.split(';').collect::<Vec<_>>();
    if fields.get(0) != Some(&"777")
        || fields.get(1) != Some(&"LiteTerm")
        || fields.get(2) != Some(&session.token())
        || fields.get(3)?.parse::<u64>().ok()? != session.generation
    {
        return None;
    }

    match fields.as_slice() {
        [_, _, _, _, "P"] => Some(MarkerKind::Prompt),
        [_, _, _, _, "H", payload] => {
            let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .ok()?;
            if base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&decoded) != *payload {
                return None;
            }
            let path = std::str::from_utf8(&decoded).ok()?;
            if path.is_empty() || !path.starts_with('/') || path.chars().any(char::is_control) {
                return None;
            }
            Some(MarkerKind::HistoryPath(path.to_owned()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const GENERATION: u64 = 42;

    fn session() -> CompletionSessionKey {
        CompletionSessionKey::new_for_test(GENERATION, TOKEN)
    }

    fn prompt_body() -> String {
        format!("777;LiteTerm;{TOKEN};{GENERATION};P")
    }

    fn history_body(path: &[u8]) -> String {
        format!(
            "777;LiteTerm;{TOKEN};{GENERATION};H;{}",
            URL_SAFE_NO_PAD.encode(path)
        )
    }

    fn bel_frame(body: impl AsRef<[u8]>) -> Vec<u8> {
        let mut frame = b"\x1b]".to_vec();
        frame.extend_from_slice(body.as_ref());
        frame.push(b'\x07');
        frame
    }

    fn st_frame(body: impl AsRef<[u8]>) -> Vec<u8> {
        let mut frame = b"\x1b]".to_vec();
        frame.extend_from_slice(body.as_ref());
        frame.extend_from_slice(b"\x1b\\");
        frame
    }

    #[test]
    fn bel_marker_reports_chunk_exclusive_end_offset() {
        let mut decoder = MarkerDecoder::new(session());
        let frame = bel_frame(prompt_body());
        let mut chunk = b"ordinary-before".to_vec();
        chunk.extend_from_slice(&frame);
        let expected_end = chunk.len();
        chunk.extend_from_slice(b"ordinary-after");

        assert_eq!(
            decoder.scan(&chunk),
            vec![MarkerBoundary {
                end_offset: expected_end,
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn st_split_across_chunks_decodes_absolute_history_path() {
        let mut decoder = MarkerDecoder::new(session());
        let path = b"/home/test/.bash_history";
        let frame = st_frame(history_body(path));
        let split = frame.len() - 1;
        let mut first = b"before".to_vec();
        first.extend_from_slice(&frame[..split]);

        assert!(decoder.scan(&first).is_empty());
        assert_eq!(
            decoder.scan(&[frame[split], b'a', b'f', b't', b'e', b'r']),
            vec![MarkerBoundary {
                end_offset: 1,
                kind: MarkerKind::HistoryPath("/home/test/.bash_history".to_owned()),
            }]
        );
    }

    #[test]
    fn invalid_and_oversized_frames_are_ignored_then_decoder_recovers() {
        let mut decoder = MarkerDecoder::new(session());
        let wrong_token = bel_frame(format!(
            "777;LiteTerm;ffffffffffffffffffffffffffffffff;{GENERATION};P"
        ));
        let wrong_generation = bel_frame(format!("777;LiteTerm;{TOKEN};{};P", GENERATION + 1));
        let oversized = bel_frame(vec![b'x'; MAX_OSC_FRAME + 1]);
        let valid = bel_frame(prompt_body());
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&wrong_token);
        chunk.extend_from_slice(&wrong_generation);
        chunk.extend_from_slice(&oversized);
        let valid_start = chunk.len();
        chunk.extend_from_slice(&valid);

        assert_eq!(
            decoder.scan(&chunk),
            vec![MarkerBoundary {
                end_offset: valid_start + valid.len(),
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn multiple_markers_in_one_chunk_have_individual_offsets() {
        let mut decoder = MarkerDecoder::new(session());
        let prompt = bel_frame(prompt_body());
        let history = st_frame(history_body(b"/tmp/history"));
        let mut chunk = b"x".to_vec();
        chunk.extend_from_slice(&prompt);
        let prompt_end = chunk.len();
        chunk.extend_from_slice(b"middle");
        chunk.extend_from_slice(&history);
        let history_end = chunk.len();
        chunk.push(b'y');

        assert_eq!(
            decoder.scan(&chunk),
            vec![
                MarkerBoundary {
                    end_offset: prompt_end,
                    kind: MarkerKind::Prompt,
                },
                MarkerBoundary {
                    end_offset: history_end,
                    kind: MarkerKind::HistoryPath("/tmp/history".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn marker_survives_every_byte_boundary_between_chunks() {
        for frame in [
            bel_frame(prompt_body()),
            st_frame(history_body(b"/var/tmp/bash-history")),
        ] {
            for split in 0..=frame.len() {
                let mut decoder = MarkerDecoder::new(session());
                let first = decoder.scan(&frame[..split]);
                let second = decoder.scan(&frame[split..]);
                let markers = first
                    .into_iter()
                    .chain(second)
                    .collect::<Vec<MarkerBoundary>>();
                let expected_kind = if frame[frame.len() - 1] == b'\x07' {
                    MarkerKind::Prompt
                } else {
                    MarkerKind::HistoryPath("/var/tmp/bash-history".to_owned())
                };
                let expected_offset = if split == frame.len() {
                    frame.len()
                } else {
                    frame.len() - split
                };

                assert_eq!(
                    markers,
                    vec![MarkerBoundary {
                        end_offset: expected_offset,
                        kind: expected_kind,
                    }],
                    "split at {split} of {}",
                    frame.len()
                );
            }
        }
    }

    #[test]
    fn malformed_frames_and_unsafe_paths_are_rejected() {
        let malformed_utf8 = bel_frame(
            [
                format!("777;LiteTerm;{TOKEN};{GENERATION};P").as_bytes(),
                &[0xff],
            ]
            .concat(),
        );
        let malformed_base64 = bel_frame(format!("777;LiteTerm;{TOKEN};{GENERATION};H;%%%"));
        let malformed_path_utf8 = bel_frame(format!(
            "777;LiteTerm;{TOKEN};{GENERATION};H;{}",
            URL_SAFE_NO_PAD.encode([0xff])
        ));
        let relative_path = bel_frame(history_body(b"relative/history"));
        let empty_path = bel_frame(history_body(b""));
        let control_path = bel_frame(history_body(b"/tmp/\nsecret"));
        let prompt_extra = bel_frame(format!("777;LiteTerm;{TOKEN};{GENERATION};P;extra"));
        let history_extra = bel_frame(format!(
            "777;LiteTerm;{TOKEN};{GENERATION};H;{};extra",
            URL_SAFE_NO_PAD.encode(b"/tmp/history")
        ));
        let control_frame = bel_frame(format!(
            "777;LiteTerm;{TOKEN};{GENERATION};H;\n{}",
            URL_SAFE_NO_PAD.encode(b"/tmp/history")
        ));
        let valid = bel_frame(prompt_body());
        let mut chunk = Vec::new();
        for invalid in [
            malformed_utf8,
            malformed_base64,
            malformed_path_utf8,
            relative_path,
            empty_path,
            control_path,
            prompt_extra,
            history_extra,
            control_frame,
        ] {
            chunk.extend_from_slice(&invalid);
        }
        let valid_start = chunk.len();
        chunk.extend_from_slice(&valid);

        assert_eq!(
            MarkerDecoder::new(session()).scan(&chunk),
            vec![MarkerBoundary {
                end_offset: valid_start + valid.len(),
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn csi_other_osc_and_plain_text_do_not_produce_markers() {
        let mut decoder = MarkerDecoder::new(session());
        let mut input = b"plain\x1b[31mred\x1b[0m".to_vec();
        input.extend_from_slice(b"\x1b]0;window title\x07");
        input.extend_from_slice(b"\x1b]776;LiteTerm;ignored\x1b\\");

        assert!(decoder.scan(&input).is_empty());
    }

    #[test]
    fn non_st_escape_is_retained_as_frame_data_and_rejected() {
        let mut decoder = MarkerDecoder::new(session());
        let mut invalid = b"\x1b]777;LiteTerm;".to_vec();
        invalid.extend_from_slice(TOKEN.as_bytes());
        invalid.extend_from_slice(format!(";{GENERATION};").as_bytes());
        invalid.extend_from_slice(b"\x1bXP\x07");
        let valid = bel_frame(prompt_body());
        let valid_start = invalid.len();
        invalid.extend_from_slice(&valid);

        assert_eq!(
            decoder.scan(&invalid),
            vec![MarkerBoundary {
                end_offset: valid_start + valid.len(),
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn oversized_frame_storage_stays_bounded_until_bel_then_recovers() {
        let mut decoder = MarkerDecoder::new(session());
        let mut exact = b"\x1b]".to_vec();
        exact.extend(std::iter::repeat_n(b'x', MAX_OSC_FRAME));
        assert!(decoder.scan(&exact).is_empty());
        assert_eq!(decoder.frame.len(), MAX_OSC_FRAME);
        assert!(!decoder.overflow);

        assert!(decoder.scan(b"x").is_empty());
        assert!(decoder.overflow);
        assert!(decoder.frame.is_empty());

        assert!(decoder.scan(&vec![b'x'; MAX_OSC_FRAME * 4]).is_empty());
        assert!(decoder.overflow);
        assert!(decoder.frame.is_empty());

        let valid = bel_frame(prompt_body());
        let mut reset_and_valid = b"\x07".to_vec();
        reset_and_valid.extend_from_slice(&valid);
        assert_eq!(
            decoder.scan(&reset_and_valid),
            vec![MarkerBoundary {
                end_offset: 1 + valid.len(),
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn oversized_frame_resets_on_split_st_then_recovers() {
        let mut decoder = MarkerDecoder::new(session());
        let mut oversized = b"\x1b]".to_vec();
        oversized.extend(std::iter::repeat_n(b'x', MAX_OSC_FRAME + 1));
        oversized.push(b'\x1b');
        assert!(decoder.scan(&oversized).is_empty());
        assert!(decoder.overflow);

        assert!(decoder.scan(b"\\").is_empty());
        assert!(!decoder.in_osc);
        assert!(!decoder.overflow);

        let valid = bel_frame(prompt_body());
        assert_eq!(
            decoder.scan(&valid),
            vec![MarkerBoundary {
                end_offset: valid.len(),
                kind: MarkerKind::Prompt,
            }]
        );
    }
}
