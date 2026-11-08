use super::*;
use flate2::{Compression, write::ZlibEncoder};
use std::io::{Cursor, Write};

#[test]
fn size_summary_preserves_exact_bytes_alongside_decimal_units() {
    assert_eq!(file_size(0), "0 B (0)");
    assert_eq!(file_size(999), "999 B (999)");
    assert_eq!(file_size(1000), "1 KB (1,000)");
    assert_eq!(file_size(423_322), "423.32 KB (423,322)");
    assert_eq!(file_size(3_000_000), "3 MB (3,000,000)");
    assert_eq!(file_size(3_234_567), "3.23 MB (3,234,567)");
    assert!(file_size(u64::MAX).ends_with("(18,446,744,073,709,551,615)"));
}

#[test]
fn apng_reports_declared_frames_without_decoding_pixel_data() {
    let png = with_chunks(vec![chunk(b"acTL", &[0, 0, 0, 22, 0, 0, 0, 3])]);
    let mut report = Metadata::default();
    super::png(Cursor::new(&png), png.len() as u64, &mut report).unwrap();
    assert!(report.fields.contains(&("Frame count".into(), "22".into())));
    assert!(
        report
            .fields
            .contains(&("APNG plays (0 = forever)".into(), "3".into()))
    );
    let mut still = Metadata::default();
    let png = with_chunks(vec![]);
    super::png(Cursor::new(&png), png.len() as u64, &mut still).unwrap();
    assert!(!still.fields.iter().any(|(key, _)| key == "Frame count"));
    assert!(png_field(b"acTL", &[0; 8], &mut Metadata::default()).is_err());
}

fn compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut crc = Crc::new();
    crc.update(tag);
    crc.update(payload);
    [
        &(payload.len() as u32).to_be_bytes()[..],
        tag,
        payload,
        &crc.sum().to_be_bytes(),
    ]
    .concat()
}

fn with_chunks(chunks: Vec<Vec<u8>>) -> Vec<u8> {
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(4, 3)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let mut png = png.into_inner();
    let end = png.split_off(png.len() - 12); // All inserted chunks come after IDAT.
    for c in chunks {
        png.extend(c);
    }
    png.extend(end);
    png
}

#[test]
fn png_keeps_duplicate_fields_and_all_prompt_text_encodings_after_pixels() {
    let workflow = format!("{{\"positive\":\"{}\"}}", "별밤 ".repeat(3500));
    let zipped = compress(workflow.as_bytes());
    let utf8 = "한글 프롬프트\n🌙";
    let png = with_chunks(vec![
        chunk(b"tEXt", b"parameters\0caf\xe9\nSteps: 20"),
        chunk(b"tEXt", b"parameters\0another prompt"),
        chunk(
            b"zTXt",
            &[&b"note\0\x00"[..], &compress(b"Cr\xe8me")].concat(),
        ),
        chunk(
            b"iTXt",
            &[&b"workflow\0\x01\x00en\0graph\0"[..], &zipped].concat(),
        ),
        chunk(
            b"iTXt",
            &[
                &b"prompt\0\x00\x00ko\0"[..],
                "제목".as_bytes(),
                b"\0",
                utf8.as_bytes(),
            ]
            .concat(),
        ),
    ]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("generated.png");
    fs::write(&path, &png).unwrap();
    let report = read(&path).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let expected = [
        ("Dimensions", "4 × 3 px"),
        ("PNG tEXt · parameters", "café\nSteps: 20"),
        ("PNG tEXt · parameters", "another prompt"),
        ("PNG zTXt · note", "Crème"),
        ("PNG iTXt · workflow", workflow.as_str()),
        ("PNG iTXt · prompt · language", "ko"),
        ("PNG iTXt · prompt · translated keyword", "제목"),
        ("PNG iTXt · prompt", utf8),
    ];
    for (k, v) in expected {
        assert!(
            report.fields.iter().any(|(key, val)| key == k && val == v),
            "{k}"
        );
    }
    assert_eq!(fs::read(path).unwrap(), png);
}

#[test]
fn oversized_and_malformed_text_reports_limits_and_preserves_later_fields() {
    let bomb = compress(&vec![b'a'; FIELD_LIMIT + 1]);
    let bytes = with_chunks(vec![
        chunk(b"zTXt", &[&b"workflow\0\0"[..], &bomb].concat()),
        chunk(b"iTXt", b"bad\0\xFF\0"),
        chunk(b"tEXt", b"software\0still readable"),
    ]);
    let mut out = Metadata::default();
    png(Cursor::new(&bytes), bytes.len() as u64, &mut out).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("truncated")));
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("compression settings"))
    );
    assert!(
        out.fields
            .iter()
            .any(|(k, v)| k == "PNG tEXt · software" && v == "still readable")
    );
    assert!(
        out.fields
            .iter()
            .any(|(k, v)| k == "PNG zTXt · workflow" && v.len() == FIELD_LIMIT)
    );
}
