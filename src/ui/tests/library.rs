use super::*;
use crate::core::library::{Entry, Kind};
use crate::i18n::tr;
use std::path::{Path, PathBuf};

fn store(window: &TestWindow) -> Store {
    Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    )
}

fn bookmarks(state: &Store) -> Vec<Entry> {
    state
        .library()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.kind == Kind::Bookmark)
        .collect()
}

fn row(window: &TestWindow, kind: &str, folder: &Path) -> Option<egui::Response> {
    window
        .ctx
        .read_response(egui::Id::unique((format!("{kind}-folder"), folder)))
}

fn image_folder(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir(&dir).unwrap();
    image::RgbImage::from_pixel(8, 4, image::Rgb([220, 50, 50]))
        .save(dir.join("1.png"))
        .unwrap();
    crate::core::paths::canonicalize(&dir).unwrap()
}

fn is_thumbnail(window: &TestWindow, id: egui::TextureId) -> bool {
    window
        .ctx
        .tex_manager()
        .read()
        .meta(id)
        .is_some_and(|meta| meta.name.starts_with("library-thumbnail-"))
}

#[test]
fn b_bookmarks_the_shown_image_under_a_name_the_user_types() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::ArrowRight));
    window.wait_for("2 / 5");
    window.draw(Some(Key::B));
    let dir = crate::core::paths::canonicalize(window._images.as_ref().unwrap().path()).unwrap();
    let folder = dir.file_name().unwrap().to_string_lossy().into_owned();
    let text = window.draw(None).0;
    assert!(text.contains(tr("add-bookmark")), "{text}");
    assert!(
        text.contains(&folder),
        "The folder name is the default: {text}"
    );
    // The suggestion is selected, so typing replaces it.
    window.draw_events(vec![Event::Text("Mountains".into())], egui::Modifiers::NONE);
    // Arrow keys edit the title instead of turning pages.
    window.draw(Some(Key::ArrowRight));
    let text = window.draw(None).0;
    assert!(
        text.contains("Mountains") && !text.contains(&format!("{folder}Mountains")),
        "{text}"
    );
    window.draw(Some(Key::Enter));
    window.draw(None);
    let state = store(&window);
    let saved = bookmarks(&state);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].title(), "Mountains");
    assert_eq!(saved[0].folder, dir);
    assert_eq!(saved[0].image, Some(dir.join("02 Pelagic.png")));

    // The thumbnail is written to the user's state folder in the background.
    let deadline = Instant::now() + Duration::from_secs(8);
    let thumbnail = loop {
        window.draw(None);
        if let Some(key) = bookmarks(&state)[0].thumbnail.clone() {
            break crate::core::library::thumbnail_file(&state.thumbnail_dir(), &key);
        }
        assert!(Instant::now() < deadline, "Thumbnail was not recorded");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(thumbnail.is_file());
    assert!(thumbnail.starts_with(window._storage.path()));
    window.reveal_controls();
    window.wait_for("2 / 5");

    // Adding the same folder again says so and keeps a single bookmark.
    window.draw(Some(Key::B));
    let text = window.draw(None).0;
    assert!(
        text.contains(tr("this-folder-is-already-bookmarked")),
        "{text}"
    );
    assert!(text.contains("Mountains"), "{text}");
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .contains(&egui::ViewportCommand::Close)
    );
    // The earlier toast may still mention the bookmark; the dialog notice may not.
    window.draw(None);
    let text = window.draw(None).0;
    assert!(
        !text.contains(tr("this-folder-is-already-bookmarked")),
        "{text}"
    );
    assert_eq!(bookmarks(&state).len(), 1);

    // A new window lists it with its thumbnail and opens it at its image.
    window.app = Viewer::new(
        &CreationContext::_new_kittest(window.ctx.clone()),
        None,
        Ok(state),
    );
    for _ in 0..4 {
        window.draw(None);
    }
    let bookmark = row(&window, "bookmark", &dir).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !window.paint.iter().any(|shape| {
        matches!(&shape.shape, Shape::Mesh(mesh)
            if is_thumbnail(&window, mesh.texture_id)
                && bookmark.rect.contains_rect(mesh.calc_bounds()))
    }) {
        assert!(Instant::now() < deadline, "Thumbnail was not shown");
        std::thread::sleep(Duration::from_millis(10));
        window.draw(None);
    }
    window.click(bookmark.rect.center());
    window.reveal_controls();
    window.wait_for("2 / 5");
}

#[test]
fn bottom_bar_bookmark_sits_left_of_move_and_needs_an_image() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    let bookmark = window.titlebar_icon("bookmark");
    let moving = window.titlebar_icon("move");
    assert!(bookmark.rect.right() <= moving.rect.left());
    assert!((bookmark.rect.center().y - moving.rect.center().y).abs() < 1.);
    window.click(bookmark.rect.center());
    assert!(window.draw(None).0.contains(tr("add-bookmark")));

    let mut empty = TestWindow::open(None);
    for _ in 0..4 {
        empty.draw(None);
    }
    empty.draw(Some(Key::Escape));
    empty.draw(Some(Key::B));
    assert!(!empty.draw(None).0.contains(tr("add-bookmark")));
}

#[test]
fn narrow_library_switches_lists_with_tabs_and_edits_bookmarks() {
    let mut window = TestWindow::open(None);
    let state = store(&window);
    let root = tempfile::tempdir().unwrap();
    let recent = image_folder(root.path(), "recent");
    let marked = image_folder(root.path(), "marked");
    let gone = image_folder(root.path(), "gone");
    state.record_folder(&recent, 10).unwrap();
    state.add_bookmark(&marked.join("1.png"), "Marked").unwrap();
    state.add_bookmark(&gone.join("1.png"), "Gone").unwrap();
    std::fs::remove_dir_all(&gone).unwrap();
    window.screen_size = egui::vec2(390., 844.);
    window.app = Viewer::new(
        &CreationContext::_new_kittest(window.ctx.clone()),
        None,
        Ok(state),
    );
    for _ in 0..5 {
        window.draw(None);
    }
    assert!(row(&window, "recent", &recent).is_some());
    assert!(row(&window, "bookmark", &marked).is_none());
    window.click(window.label_position(tr("bookmarks")));
    window.draw(None);
    assert!(row(&window, "recent", &recent).is_none());
    assert!(row(&window, "bookmark", &marked).is_some());
    // A missing folder stays listed until the user removes it.
    let text = window.draw(None).0;
    assert!(
        text.contains("Gone") && text.contains(tr("folder-unavailable")),
        "{text}"
    );

    let rename = window
        .ctx
        .read_response(egui::Id::unique(("bookmark-rename", &marked)))
        .unwrap();
    window.click(rename.rect.center());
    let text = window.draw(None).0;
    assert!(text.contains(tr("rename-bookmark")), "{text}");
    window.draw_events(vec![Event::Text("Trips".into())], egui::Modifiers::NONE);
    window.draw(Some(Key::Enter));
    window.draw(None);
    let state = store(&window);
    let titles: Vec<_> = bookmarks(&state).iter().map(Entry::title).collect();
    assert_eq!(titles, ["Gone", "Trips"]);
    assert!(window.draw(None).0.contains("Trips"));

    let remove = window
        .ctx
        .read_response(egui::Id::unique(("bookmark-remove", &gone)))
        .unwrap();
    window.click(remove.rect.center());
    window.draw(None);
    assert_eq!(bookmarks(&state).len(), 1);
    assert!(marked.join("1.png").is_file());
    assert!(row(&window, "bookmark", &gone).is_none());
}
