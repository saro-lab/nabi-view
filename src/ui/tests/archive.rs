use super::*;
use crate::core::library::Kind;
use crate::i18n::tr;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

const PAGES: [&str; 3] = ["ch1/1.png", "ch1/2.png", "ch2/1.png"];

fn page(shade: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbImage::from_pixel(8, 4, image::Rgb([shade, 60, 60]))
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

fn zip_archive(root: &Path, password: Option<&str>) -> PathBuf {
    use zip::write::SimpleFileOptions;
    let path = root.join("book.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let options = match password {
        Some(password) => options.with_aes_encryption(zip::AesMode::Aes256, password),
        None => options,
    };
    for (index, name) in PAGES.iter().rev().enumerate() {
        zip.start_file(*name, options).unwrap();
        zip.write_all(&page(40 * index as u8 + 40)).unwrap();
    }
    zip.start_file("notes.txt", options).unwrap();
    zip.write_all(b"not a page").unwrap();
    zip.finish().unwrap();
    crate::core::paths::canonicalize(&path).unwrap()
}

fn sevenz_archive(root: &Path, password: &str) -> PathBuf {
    use sevenz_rust2::{
        ArchiveEntry, ArchiveWriter, EncoderMethod, encoder_options::AesEncoderOptions,
    };
    let path = root.join("book.7z");
    let mut writer = ArchiveWriter::create(&path).unwrap();
    writer.set_content_methods(vec![
        AesEncoderOptions::new(password.into()).into(),
        EncoderMethod::LZMA2.into(),
    ]);
    for (index, name) in PAGES.iter().enumerate() {
        writer
            .push_archive_entry(
                ArchiveEntry::new_file(name),
                Some(Cursor::new(page(40 * index as u8 + 40))),
            )
            .unwrap();
    }
    writer.finish().unwrap();
    crate::core::paths::canonicalize(&path).unwrap()
}

fn wait_for_text(window: &mut TestWindow, label: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let text = window.draw(None).0;
        if text.contains(label) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for {label:?}. Visible labels: {text}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn store(window: &TestWindow) -> Store {
    Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    )
}

#[test]
fn an_archive_opens_like_a_folder_but_refuses_moving_deleting_and_restoring() {
    let root = tempfile::tempdir().unwrap();
    let archive = zip_archive(root.path(), None);
    let before = std::fs::read(&archive).unwrap();
    let mut window = TestWindow::open(Some(archive.clone()));
    window.reveal_controls();
    window.wait_for("1 / 3");
    let blocked = tr("not-available-in-archives");
    for key in [Key::M, Key::Delete] {
        window.draw(Some(key));
        let text = window.draw(None).0;
        assert!(text.contains(blocked), "{key:?}: {text}");
        assert!(!text.contains(tr("move-images")) || text.contains("1 / 3"));
        // Another page clears the toast before the next command.
        window.draw(Some(Key::ArrowRight));
        window.draw(Some(Key::ArrowLeft));
        assert!(!window.draw(None).0.contains(blocked));
    }
    let undo = egui::Modifiers::COMMAND;
    window.draw_events(
        vec![
            key_event(Key::Z, true, false, undo),
            key_event(Key::Z, false, false, undo),
        ],
        undo,
    );
    let text = window.draw(None).0;
    assert!(text.contains(blocked), "{text}");
    assert!(
        !text.contains(tr("there-are-no-images-to-restore")),
        "{text}"
    );

    // Pages follow their path inside the archive, and nothing was changed.
    window.wait_for("1 / 3");
    window.draw(Some(Key::ArrowRight));
    window.wait_for("2 / 3");
    window.draw(Some(Key::ArrowRight));
    window.wait_for("3 / 3");
    assert_eq!(std::fs::read(&archive).unwrap(), before);
}

#[test]
fn an_encrypted_archive_asks_for_its_password_until_it_is_right() {
    let root = tempfile::tempdir().unwrap();
    for archive in [
        zip_archive(root.path(), Some("열쇠")),
        sevenz_archive(root.path(), "열쇠"),
    ] {
        let mut window = TestWindow::open(Some(archive.clone()));
        let text = wait_for_text(&mut window, tr("archive-password"));
        assert!(!text.contains(tr("the-password-is-incorrect")), "{text}");
        // Keys type into the dialog instead of running viewer commands.
        window.draw_events(vec![Event::Text("wrong".into())], egui::Modifiers::NONE);
        window.draw(Some(Key::Enter));
        let text = wait_for_text(&mut window, tr("the-password-is-incorrect"));
        assert!(text.contains(tr("archive-password")), "{text}");
        window.draw_events(vec![Event::Text("열쇠".into())], egui::Modifiers::NONE);
        window.draw(Some(Key::Enter));
        window.reveal_controls();
        window.wait_for("1 / 3");
        let text = window.draw(None).0;
        assert!(
            !text.contains(tr("archive-password")),
            "{archive:?}: {text}"
        );
    }
}

#[test]
fn cancelling_the_password_returns_to_the_start_screen() {
    let root = tempfile::tempdir().unwrap();
    let archive = zip_archive(root.path(), Some("secret"));
    let mut window = TestWindow::open(Some(archive));
    wait_for_text(&mut window, tr("archive-password"));
    window.draw(Some(Key::Escape));
    let text = wait_for_text(&mut window, tr("recent"));
    assert!(!text.contains(tr("archive-password")), "{text}");
    // Nothing is left open, so file commands have nothing to refuse.
    window.draw(Some(Key::Escape));
    window.draw(Some(Key::Delete));
    let text = window.draw(None).0;
    assert!(!text.contains(tr("not-available-in-archives")), "{text}");
}

#[test]
fn archives_are_remembered_and_bookmarked_at_their_page() {
    let root = tempfile::tempdir().unwrap();
    let archive = zip_archive(root.path(), None);
    let saved_page = archive.join("ch1").join("2.png");
    let mut window = TestWindow::open(Some(archive.clone()));
    window.reveal_controls();
    window.wait_for("1 / 3");
    window.draw(Some(Key::ArrowRight));
    window.wait_for("2 / 3");
    window.draw(Some(Key::B));
    window.draw(None);
    window.draw(Some(Key::Enter));
    window.draw(None);
    let state = store(&window);
    let entries = state.library().unwrap();
    let bookmark = entries
        .iter()
        .find(|entry| entry.kind == Kind::Bookmark)
        .expect("bookmark");
    assert_eq!(bookmark.folder, archive);
    assert_eq!(bookmark.image, Some(saved_page.clone()));
    assert!(bookmark.available);

    // Closing the window saves the page and deletes the extracted copies.
    window.app.on_exit();
    let recent = state
        .library()
        .unwrap()
        .into_iter()
        .find(|entry| entry.kind == Kind::Recent)
        .expect("recent archive");
    assert_eq!(recent.folder, archive);
    assert_eq!(recent.image, Some(saved_page.clone()));

    // The saved page opens the archive there.
    let mut reopened = TestWindow::open(Some(saved_page));
    reopened.reveal_controls();
    reopened.wait_for("2 / 3");

    // A deleted archive leaves the recent list and marks its bookmark.
    drop(reopened);
    std::fs::remove_file(&archive).unwrap();
    let entries = state.library().unwrap();
    assert!(entries.iter().all(|entry| entry.kind != Kind::Recent));
    assert!(entries.iter().any(|entry| !entry.available), "{entries:?}");
}
