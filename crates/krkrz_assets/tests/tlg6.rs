use krkrz_assets::media::Image;
use tlg::{tlg_type::TlgEncoderTrait, tlg6::Tlg6Encoder};

#[test]
fn independent_tlg6_encoder_roundtrip_and_malformed_input() {
    for (w, h) in [(1, 1), (7, 9), (16, 17), (31, 24)] {
        let rgba: Vec<_> = (0..w * h)
            .flat_map(|i| [i as u8, (i * 3) as u8, (i * 7) as u8, (i * 11) as u8])
            .collect();
        let encoded = Tlg6Encoder::from_rgba(rgba.clone(), w, h).encode().unwrap();
        assert_eq!(
            Image::decode(&encoded).unwrap(),
            Image {
                width: w,
                height: h,
                rgba
            }
        );
        for n in 0..encoded.len() {
            assert!(
                Image::decode(&encoded[..n]).is_err(),
                "truncation {w}x{h} at {n}"
            );
        }
        for offset in [11, 12, 15, 19, 23, 27] {
            let mut bad = encoded.clone();
            bad[offset..offset + if offset >= 15 { 4 } else { 1 }].fill(255);
            assert!(Image::decode(&bad).is_err(), "bad header {offset}");
        }
    }
}

#[test]
fn tlg6_rgb_and_grayscale() {
    let rgb = vec![25, 77, 130, 255, 0, 128];
    let encoded = Tlg6Encoder::from_rgb(rgb, 2, 1).encode().unwrap();
    assert_eq!(
        Image::decode(&encoded).unwrap().rgba,
        [25, 77, 130, 255, 255, 0, 128, 255]
    );
    let encoded = Tlg6Encoder::from_gray(vec![12, 40], 2, 1).encode().unwrap();
    assert_eq!(
        Image::decode(&encoded).unwrap().rgba,
        [12, 12, 12, 255, 40, 40, 40, 255]
    );
}
