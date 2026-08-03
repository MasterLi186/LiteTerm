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
