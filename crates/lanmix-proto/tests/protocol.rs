use lanmix_proto::{
    HEADER_BYTES, Header, MAX_DATAGRAM_BYTES, MAX_FRAMES, Packet, ProtocolError, StereoFrame,
    decode, encode,
};

fn header(frame_count: u16) -> Header {
    Header {
        session_id: 0x0807_0605_0403_0201,
        stream_id: 0x0c0b_0a09,
        seq: 0x100f_0e0d,
        frame_index: 0x1817_1615_1413_1211,
        sample_rate: 48_000,
        frame_count,
    }
}

fn encoded_one() -> [u8; 48] {
    let mut bytes = [0; 48];
    assert_eq!(encode(&header(1), &[[1.0, -2.0]], &mut bytes), Ok(48));
    bytes
}

fn assert_decode_error(bytes: &[u8], expected: ProtocolError) {
    assert_eq!(decode(bytes).unwrap_err(), expected);
}

#[test]
fn exact_little_endian_wire_bytes() {
    // Independent golden vector: every multi-byte header field has distinct
    // bytes, and the payload spells out the IEEE 754 encodings for 1 and -2.
    let expected = [
        0x4c, 0x4d, 0x01, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
        0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x80, 0xbb,
        0x00, 0x00, 0x01, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3f, 0x00,
        0x00, 0x00, 0xc0,
    ];
    assert_eq!(encoded_one(), expected);
    let packet = decode(&expected).unwrap();
    assert_eq!(packet.header, header(1));
    assert_eq!(packet.frames().collect::<Vec<_>>(), [[1.0, -2.0]]);
}

#[test]
fn every_truncation_is_rejected() {
    let bytes = encoded_one();
    for len in 0..bytes.len() {
        assert!(decode(&bytes[..len]).is_err(), "accepted length {len}");
    }
    assert_decode_error(&[], ProtocolError::TruncatedHeader { actual: 0 });
    assert_decode_error(
        &bytes[..47],
        ProtocolError::InvalidLength {
            expected: 48,
            actual: 47,
        },
    );
}

#[test]
fn trailing_bytes_and_oversized_datagrams_are_rejected() {
    let mut bytes = encoded_one().to_vec();
    bytes.push(0);
    assert_decode_error(
        &bytes,
        ProtocolError::InvalidLength {
            expected: 48,
            actual: 49,
        },
    );
    assert_decode_error(
        &[0; MAX_DATAGRAM_BYTES + 1],
        ProtocolError::DatagramTooLarge {
            actual: MAX_DATAGRAM_BYTES + 1,
        },
    );
}

#[test]
fn fixed_fields_are_strict() {
    for offset in 0..2 {
        let mut bytes = encoded_one();
        bytes[offset] ^= 1;
        assert_decode_error(&bytes, ProtocolError::InvalidMagic);
    }
    for value in 0..=u8::MAX {
        for (offset, valid, error) in [
            (2, 1, ProtocolError::UnsupportedVersion(value)),
            (3, 0, ProtocolError::InvalidFlags(value)),
            (34, 2, ProtocolError::InvalidChannels(value)),
            (35, 1, ProtocolError::UnsupportedFormat(value)),
        ] {
            if value != valid {
                let mut bytes = encoded_one();
                bytes[offset] = value;
                assert_decode_error(&bytes, error);
            }
        }
    }
    for bit in 0..32 {
        let mut bytes = encoded_one();
        let reserved = 1u32 << bit;
        bytes[36..40].copy_from_slice(&reserved.to_le_bytes());
        assert_decode_error(&bytes, ProtocolError::InvalidReserved(reserved));
    }
}

#[test]
fn sample_rate_boundaries_are_enforced_by_both_directions() {
    for rate in [0, 7_999, 8_000, 44_100, 192_000, 192_001, u32::MAX] {
        let mut h = header(1);
        h.sample_rate = rate;
        let mut bytes = encoded_one();
        bytes[28..32].copy_from_slice(&rate.to_le_bytes());
        let mut out = [0xa5; 48];
        let result = encode(&h, &[[1.0, -2.0]], &mut out);
        if (8_000..=192_000).contains(&rate) {
            assert_eq!(result, Ok(48));
            assert_eq!(decode(&bytes).unwrap().header.sample_rate, rate);
        } else {
            assert_eq!(result, Err(ProtocolError::InvalidSampleRate(rate)));
            assert_eq!(out, [0xa5; 48]);
            assert_decode_error(&bytes, ProtocolError::InvalidSampleRate(rate));
        }
    }
}

#[test]
fn frame_counts_and_output_capacity_are_checked_before_writing() {
    let mut out = [0xa5; MAX_DATAGRAM_BYTES];
    assert_eq!(
        encode(&header(2), &[[0.0; 2]], &mut out),
        Err(ProtocolError::FrameCountMismatch {
            header: 2,
            actual: 1
        }),
    );
    for count in [146, 256, u16::MAX] {
        assert_eq!(
            encode(&header(count), &[], &mut out),
            Err(ProtocolError::TooManyFrames {
                actual: usize::from(count)
            }),
        );
        let mut bytes = encoded_one();
        bytes[32..34].copy_from_slice(&count.to_le_bytes());
        assert_decode_error(
            &bytes,
            ProtocolError::TooManyFrames {
                actual: usize::from(count),
            },
        );
    }
    assert_eq!(out, [0xa5; MAX_DATAGRAM_BYTES]);
    for len in 0..48 {
        assert_eq!(
            encode(&header(1), &[[0.0; 2]], &mut out[..len]),
            Err(ProtocolError::BufferTooSmall {
                required: 48,
                available: len
            }),
        );
        assert_eq!(out, [0xa5; MAX_DATAGRAM_BYTES]);
    }
    let mut bytes = encoded_one();
    bytes[32..34].copy_from_slice(&0u16.to_le_bytes());
    assert_decode_error(
        &bytes,
        ProtocolError::InvalidLength {
            expected: 40,
            actual: 48,
        },
    );
}

#[test]
fn nonfinite_samples_are_rejected_at_every_position() {
    for bits in [
        0x7fc0_0000,
        0x7f80_0001,
        0xffc0_0000,
        0x7f80_0000,
        0xff80_0000,
    ] {
        for frame in 0..3 {
            for channel in 0..2 {
                let mut samples = [[0.0; 2]; 3];
                let mut bytes = [0; 64];
                encode(&header(3), &samples, &mut bytes).unwrap();
                samples[frame][channel] = f32::from_bits(bits);
                let offset = HEADER_BYTES + frame * 8 + channel * 4;
                bytes[offset..offset + 4].copy_from_slice(&bits.to_le_bytes());
                let expected = ProtocolError::NonFiniteSample { frame, channel };
                assert_decode_error(&bytes, expected);
                let mut out = [0xa5; 64];
                assert_eq!(encode(&header(3), &samples, &mut out), Err(expected));
                assert_eq!(out, [0xa5; 64]);
            }
        }
    }
}

#[test]
fn maximum_payload_round_trip_preserves_sample_bits() {
    assert_eq!(HEADER_BYTES, 40);
    assert_eq!(MAX_DATAGRAM_BYTES, 1200);
    assert_eq!(MAX_FRAMES, 145);
    let special = [
        0.0,
        -0.0,
        f32::MAX,
        f32::MIN,
        f32::MIN_POSITIVE,
        f32::from_bits(1),
    ];
    let samples: [StereoFrame; MAX_FRAMES] = core::array::from_fn(|i| {
        if i < special.len() {
            [special[i], -special[i]]
        } else {
            [i as f32 / 7.0, -(i as f32)]
        }
    });
    let mut h = header(MAX_FRAMES as u16);
    h.seq = u32::MAX;
    h.frame_index = u64::MAX - u64::from(h.frame_count);
    let mut bytes = [0; MAX_DATAGRAM_BYTES];
    assert_eq!(encode(&h, &samples, &mut bytes), Ok(MAX_DATAGRAM_BYTES));
    let packet = decode(&bytes).unwrap();
    assert_eq!(packet.header, h);
    let mut iter = packet.frames();
    assert_eq!(iter.len(), MAX_FRAMES);
    for (i, expected) in samples.iter().enumerate() {
        assert_eq!(
            iter.next().unwrap().map(f32::to_bits),
            expected.map(f32::to_bits)
        );
        assert_eq!(iter.len(), MAX_FRAMES - i - 1);
    }
    assert_eq!(iter.next(), None);
}

#[test]
fn every_legal_frame_count_round_trips_at_the_end_index_boundary() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<Header>();

    let samples: [StereoFrame; MAX_FRAMES] =
        core::array::from_fn(|i| [i as f32 / 7.0, -(i as f32)]);
    for count in 0..=MAX_FRAMES {
        for rate in [8_000, 48_000, 192_000] {
            let mut h = header(count as u16);
            h.sample_rate = rate;
            h.session_id = u64::MAX;
            h.stream_id = u32::MAX;
            h.seq = u32::MAX;
            h.frame_index = u64::MAX - u64::from(h.frame_count);
            let mut bytes = [0xa5; MAX_DATAGRAM_BYTES];
            let len = encode(&h, &samples[..count], &mut bytes).unwrap();
            assert_eq!(len, HEADER_BYTES + count * 8);
            assert!(bytes[len..].iter().all(|&b| b == 0xa5));
            let packet = decode(&bytes[..len]).unwrap();
            assert_eq!(packet.header, h);
            assert_eq!(packet.frames().len(), count);
            assert!(packet.frames().eq(samples[..count].iter().copied()));
        }
    }
}

#[test]
fn frame_index_overflow_is_rejected_by_encode_and_decode() {
    let samples = [[0.0; 2]; MAX_FRAMES];
    for count in 1..=MAX_FRAMES {
        let mut h = header(count as u16);
        let mut bytes = [0; MAX_DATAGRAM_BYTES];
        let len = encode(&h, &samples[..count], &mut bytes).unwrap();
        for index in [u64::MAX - u64::from(h.frame_count) + 1, u64::MAX] {
            h.frame_index = index;
            let expected = ProtocolError::FrameIndexOverflow {
                frame_index: index,
                frame_count: h.frame_count,
            };
            let mut out = [0xa5; MAX_DATAGRAM_BYTES];
            assert_eq!(encode(&h, &samples[..count], &mut out), Err(expected));
            assert_eq!(out, [0xa5; MAX_DATAGRAM_BYTES]);
            // Modify a previously valid wire packet independently of encode.
            bytes[20..28].copy_from_slice(&index.to_le_bytes());
            assert_decode_error(&bytes[..len], expected);
        }
    }
}

#[test]
fn empty_packet_and_extra_output_capacity() {
    let mut out = [0xa5; MAX_DATAGRAM_BYTES + 100];
    assert_eq!(encode(&header(0), &[], &mut out), Ok(HEADER_BYTES));
    assert!(out[HEADER_BYTES..].iter().all(|&b| b == 0xa5));
    let packet = decode(&out[..HEADER_BYTES]).unwrap();
    assert_eq!(packet.header, header(0));
    assert_eq!(packet.frames().len(), 0);
}

#[test]
fn borrowed_payload_is_independent_of_public_header_mutations() {
    fn borrow_packet(bytes: &[u8]) -> Packet<'_> {
        decode(bytes).unwrap()
    }
    let bytes = encoded_one();
    let mut packet = borrow_packet(&bytes);
    packet.header.frame_count = u16::MAX;
    assert_eq!(packet.frames().len(), 1);
    assert_eq!(packet.frames().next(), Some([1.0, -2.0]));
    assert_eq!(packet.frames().next(), Some([1.0, -2.0]));
}
