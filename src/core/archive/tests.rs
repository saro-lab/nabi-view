use super::*;
use std::collections::HashSet;
use std::io::Cursor;

fn png(shade: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(3, 2, image::Rgba([shade, 0, 0, 255]))
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

enum ZipKey {
    None,
    Aes(&'static str),
}

fn write_zip(path: &Path, entries: &[(&str, Vec<u8>)], key: ZipKey) {
    use zip::write::SimpleFileOptions;
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, data) in entries {
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let options = match key {
            ZipKey::None => options,
            ZipKey::Aes(password) => options.with_aes_encryption(zip::AesMode::Aes256, password),
        };
        if name.ends_with('/') {
            zip.add_directory(*name, options).unwrap();
        } else {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
    }
    zip.finish().unwrap();
}

fn write_7z(path: &Path, entries: &[(&str, Vec<u8>)], password: Option<&str>, header: bool) {
    use sevenz_rust2::{
        ArchiveEntry, ArchiveWriter, EncoderMethod, encoder_options::AesEncoderOptions,
    };
    let mut writer = ArchiveWriter::create(path).unwrap();
    if let Some(password) = password {
        writer.set_content_methods(vec![
            AesEncoderOptions::new(password.into()).into(),
            EncoderMethod::LZMA2.into(),
        ]);
        writer.set_encrypt_header(header);
    }
    for (name, data) in entries {
        writer
            .push_archive_entry(
                ArchiveEntry::new_file(name),
                Some(Cursor::new(data.clone())),
            )
            .unwrap();
    }
    writer.finish().unwrap();
}

fn names(extracted: &Extracted) -> Vec<String> {
    extracted
        .pages
        .iter()
        .map(|page| page.inner.to_string_lossy().replace('\\', "/"))
        .collect()
}

fn book() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("chapter 10/2.png", png(4)),
        ("notes.txt", b"not an image".to_vec()),
        ("chapter 2/", Vec::new()),
        ("chapter 2/10.png", png(3)),
        ("chapter 2/9.png", png(2)),
        ("cover.png", png(1)),
    ]
}

#[test]
fn recognises_archives_and_splits_page_paths() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("Book.ZIP");
    write_zip(&archive, &book(), ZipKey::None);
    assert!(is_archive(&archive));
    assert!(is_archive(Path::new("a.7z")));
    assert!(!is_archive(Path::new("a.rar")));
    assert_eq!(split(&archive), None);
    assert_eq!(
        split(&archive.join("chapter 2").join("9.png")),
        Some((archive.clone(), Path::new("chapter 2").join("9.png")))
    );
    // A folder that is only named like an archive is a folder.
    let folder = root.path().join("folder.zip");
    fs::create_dir(&folder).unwrap();
    assert_eq!(split(&folder.join("a.png")), None);
    assert_eq!(split(&root.path().join("a.png")), None);
}

#[test]
fn zip_images_are_extracted_in_natural_order_and_removed_on_drop() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("book.zip");
    write_zip(&archive, &book(), ZipKey::None);
    let extracted = extract(&archive, None, &|| true).unwrap();
    assert_eq!(
        names(&extracted),
        [
            "chapter 2/9.png",
            "chapter 2/10.png",
            "chapter 10/2.png",
            "cover.png"
        ]
    );
    assert_eq!(extracted.skipped, 0);
    for page in &extracted.pages {
        assert!(page.file.starts_with(extracted.root()));
        assert_eq!(page.file.file_name(), page.inner.file_name());
    }
    assert_eq!(fs::read(&extracted.pages[3].file).unwrap(), png(1));
    // Pages of different folders with the same name stay apart.
    assert_ne!(
        extracted.pages[0].file.parent(),
        extracted.pages[2].file.parent()
    );
    assert_eq!(
        first_page(&archive),
        Some(Path::new("chapter 2").join("9.png"))
    );
    let dir = extracted.root().to_owned();
    drop(extracted);
    assert!(!dir.exists());
}

#[test]
fn entry_names_cannot_leave_the_extraction_folder() {
    assert_eq!(inner_path("../evil.png"), None);
    assert_eq!(inner_path("a/../../evil.png"), None);
    assert_eq!(
        inner_path("/abs\\dir//x.png"),
        Some(Path::new("abs").join("dir").join("x.png"))
    );
    assert_eq!(inner_path("./"), None);
    assert_eq!(safe_name("a:b?.png"), "a_b_.png");
    assert_eq!(safe_name("CON.png"), "_CON.png");
    assert_eq!(safe_name("trailing. .png"), "trailing.png");
    assert_eq!(safe_name(&format!("{}.png", "x".repeat(300))).len(), 124);

    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("names.7z");
    write_7z(
        &archive,
        &[
            ("../escape.png", png(1)),
            ("same.png", png(2)),
            ("SAME.png", png(3)),
            ("a:b.png", png(4)),
        ],
        None,
        false,
    );
    let extracted = extract(&archive, None, &|| true).unwrap();
    assert_eq!(names(&extracted), ["a:b.png", "SAME.png", "same.png"]);
    let files: HashSet<_> = extracted.pages.iter().map(|p| p.file.clone()).collect();
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|file| file.starts_with(extracted.root())));
    assert!(!root.path().join("escape.png").exists());
}

#[test]
fn encrypted_zips_ask_for_a_password_and_reject_wrong_ones() {
    let root = tempfile::tempdir().unwrap();
    // The writer only makes AES zips; legacy ZipCrypto takes the same path when read.
    for (name, key) in [("aes.zip", ZipKey::Aes("secret"))] {
        let archive = root.path().join(name);
        write_zip(&archive, &book(), key);
        assert_eq!(
            extract(&archive, None, &|| true).err(),
            Some(Error::PasswordRequired),
            "{name}"
        );
        assert_eq!(
            extract(&archive, Some("wrong"), &|| true).err(),
            Some(Error::WrongPassword),
            "{name}"
        );
        let extracted = extract(&archive, Some("secret"), &|| true).unwrap();
        assert_eq!(extracted.pages.len(), 4, "{name}");
        assert_eq!(fs::read(&extracted.pages[3].file).unwrap(), png(1));
    }
}

#[test]
fn sevenz_archives_open_with_and_without_passwords() {
    let root = tempfile::tempdir().unwrap();
    let entries = [
        ("b/1.png", png(2)),
        ("a.png", png(1)),
        ("readme.md", b"x".to_vec()),
    ];
    let plain = root.path().join("plain.7z");
    write_7z(&plain, &entries, None, false);
    let extracted = extract(&plain, None, &|| true).unwrap();
    assert_eq!(names(&extracted), ["a.png", "b/1.png"]);
    assert_eq!(first_page(&plain), Some(PathBuf::from("a.png")));
    let page = extract_page(&plain, &Path::new("b").join("1.png")).unwrap();
    assert_eq!(fs::read(&page.pages[0].file).unwrap(), png(2));
    assert!(extract_page(&plain, Path::new("missing.png")).is_err());

    for header in [false, true] {
        let archive = root.path().join(format!("locked-{header}.7z"));
        write_7z(&archive, &entries, Some("비밀"), header);
        assert_eq!(
            extract(&archive, None, &|| true).err(),
            Some(Error::PasswordRequired),
            "header {header}"
        );
        assert_eq!(
            extract(&archive, Some("wrong"), &|| true).err(),
            Some(Error::WrongPassword),
            "header {header}"
        );
        let extracted = extract(&archive, Some("비밀"), &|| true).unwrap();
        assert_eq!(names(&extracted), ["a.png", "b/1.png"], "header {header}");
        assert_eq!(fs::read(&extracted.pages[0].file).unwrap(), png(1));
    }
}

#[test]
fn extraction_stops_when_cancelled_and_reports_broken_archives() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("book.zip");
    write_zip(&archive, &book(), ZipKey::None);
    assert_eq!(
        extract(&archive, None, &|| false).err(),
        Some(Error::Cancelled)
    );
    let broken = root.path().join("broken.7z");
    fs::write(&broken, b"not an archive").unwrap();
    assert!(matches!(
        extract(&broken, None, &|| true),
        Err(Error::Failed(_))
    ));
    assert!(matches!(
        extract(&root.path().join("missing.zip"), None, &|| true),
        Err(Error::Failed(_))
    ));
}

#[test]
fn oversized_pages_are_skipped() {
    let root = tempfile::tempdir().unwrap();
    let mut writer = Writer::new(root.path(), &|| true);
    writer
        .add(
            PathBuf::from("big.png"),
            MAX_PAGE_BYTES + 1,
            None,
            &mut &b"x"[..],
        )
        .unwrap();
    writer
        .add(PathBuf::from("empty.png"), 0, None, &mut &b""[..])
        .unwrap();
    writer
        .add(PathBuf::from("ok.png"), 1, None, &mut &b"x"[..])
        .unwrap();
    assert_eq!(writer.skipped, 2);
    assert_eq!(writer.pages.len(), 1);
}

#[test]
fn archive_sort_keys_use_entry_dates_and_decoded_entry_sizes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("dates.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let payload = png(20);
    for (name, year) in [("2.png", 2024), ("10.png", 2020)] {
        let options = zip::write::SimpleFileOptions::default()
            .last_modified_time(zip::DateTime::from_date_and_time(year, 1, 2, 3, 4, 6).unwrap());
        zip.start_file(name, options).unwrap();
        zip.write_all(&payload).unwrap();
    }
    zip.finish().unwrap();
    let pages = extract(&path, None, &|| true).unwrap();
    assert_eq!(pages.pages[0].inner, Path::new("2.png"));
    assert!(pages.pages[0].sort.modified > pages.pages[1].sort.modified);
    assert!(
        pages
            .pages
            .iter()
            .all(|page| page.sort.bytes == payload.len() as u64)
    );
}
