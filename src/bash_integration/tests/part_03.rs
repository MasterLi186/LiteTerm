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
