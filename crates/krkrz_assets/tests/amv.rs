use krkrz_assets::amv::{AlphaEncoding, Movie};

fn movie(alpha: u8, payload: &[u8]) -> Vec<u8> {
    let mut b = b"AJPM".to_vec();
    let size = if alpha == 1 { 232u32 } else { 168 };
    for n in [0, 0, size, 0, 3, 1001, 30000] {
        b.extend(n.to_le_bytes());
    }
    b.extend(32u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend([alpha, 0, 0, 0]);
    b.resize(size as usize, 1);
    b.extend(b"FRAM");
    b.extend((12 + payload.len() as u32).to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend((-4i16).to_le_bytes());
    b.extend(7i16.to_le_bytes());
    b.extend(32u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(payload);
    let len = b.len() as u32;
    b[4..8].copy_from_slice(&len.to_le_bytes());
    b
}
#[test]
fn indexes_both_encodings_without_copying_packet_payloads() {
    let b = movie(1, &[10, 20, 30, 40]);
    let m = Movie::parse(b).unwrap();
    assert_eq!(m.header.alpha_encoding, AlphaEncoding::Dct);
    assert_eq!((m.header.fps_scale, m.header.fps_rate), (1001, 30000));
    assert_eq!(m.quantization.len(), 3);
    assert_eq!((m.packets[0].left, m.packets[0].top), (-4, 7));
    assert_eq!(m.entropy(0).unwrap(), [10, 20, 30, 40]);
    assert!(m.compressed_alpha(0).unwrap().is_none());
    assert!(m.entropy(1).is_err());
    let m = Movie::parse(movie(2, &[2, 0, 0, 0, 7, 8, 9, 10])).unwrap();
    assert_eq!(m.header.alpha_encoding, AlphaEncoding::Deflate);
    assert_eq!(m.compressed_alpha(0).unwrap().unwrap(), [7, 8]);
    assert_eq!(m.entropy(0).unwrap(), [9, 10]);
}
#[test]
fn empty_and_repeated_packets_are_not_discarded() {
    let mut b = movie(1, &[]);
    b[248..252].fill(0);
    b.extend_from_within(232..);
    let len = b.len() as u32;
    b[4..8].copy_from_slice(&len.to_le_bytes());
    let m = Movie::parse(b).unwrap();
    assert_eq!(m.packets.len(), 2);
    assert_eq!(m.packets[0].sequence, m.packets[1].sequence);
    assert_eq!(m.packets[0].width, 0);
}
#[test]
fn rejects_truncation_and_hostile_lengths_before_allocation() {
    let b = movie(1, &[1, 2, 3, 4]);
    for end in 0..b.len() {
        let mut truncated = b[..end].to_vec();
        if end >= 8 {
            truncated[4..8].copy_from_slice(&(end as u32).to_le_bytes());
        }
        assert!(Movie::parse(truncated).is_err(), "accepted length {end}");
    }
    for (offset, bytes) in [
        (0, b"NOPE".as_slice()),
        (4, &[0, 0, 0, 0]),
        (8, &[1, 0, 0, 0]),
        (12, &[0xff; 4]),
        (20, &[0; 4]),
        (24, &[0; 4]),
        (28, &[0; 4]),
        (32, &[0; 2]),
        (32, &[0xff; 4]),
        (36, &[0]),
        (232, b"NOPE"),
        (236, &[0xff; 4]),
        (248, &[1, 0]),
        (248, &[0xf0, 0xff, 0xf0, 0xff]),
    ] {
        let mut invalid = b.clone();
        invalid[offset..offset + bytes.len()].copy_from_slice(bytes);
        assert!(
            Movie::parse(invalid).is_err(),
            "accepted corruption at {offset}"
        );
    }
    assert!(Movie::parse(movie(2, &[255; 8])).is_err());
}

#[test]
fn dc_pixels_match_original_alpha_movie() {
    // Ten distinct DC differences, unit quantizers. Original DLL reference:
    // RGB 8880517, 9143689, 9472654, 9867412; alpha 153, 161, 170, 180.
    let movie = Movie::parse(include_bytes!("fixtures/amv/dc.amv").to_vec()).unwrap();
    let image = movie.decode_packet(0).unwrap().unwrap();
    for (x, y, rgb, a) in [
        (0, 0, 8880517u32, 153),
        (8, 0, 9143689, 161),
        (0, 8, 9472654, 170),
        (8, 8, 9867412, 180),
    ] {
        let at = (y * 16 + x) * 4;
        assert_eq!(
            &image.rgba[at..at + 4],
            &[(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, a]
        );
    }
}
#[test]
fn malformed_entropy_returns_error_without_panicking() {
    let movie = Movie::parse(movie(1, &[255; 4])).unwrap();
    assert!(movie.decode_packet(0).is_err());
}

#[test]
fn ac_reconstruction_stays_within_integer_idct_rounding_error() {
    // Four macroblocks with nonzero AC coefficients, positive/negative DC
    // differences and non-unit quantizers. Captured from the original DLL.
    // Its optimized IDCT and jpeg-decoder round intermediate values differently.
    let movie = Movie::parse(include_bytes!("fixtures/amv/ac.amv").to_vec()).unwrap();
    let image = movie.decode_packet(0).unwrap().unwrap();
    let expected = include_bytes!("fixtures/amv/ac.rgba");
    for (i, (actual, reference)) in image.rgba.iter().zip(expected).enumerate() {
        let tolerance = if i % 4 == 3 { 1 } else { 2 };
        assert!(
            actual.abs_diff(*reference) <= tolerance,
            "channel {i}: {actual} vs {reference}"
        );
    }
}
