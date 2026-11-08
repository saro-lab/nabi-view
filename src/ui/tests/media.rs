use super::*;

#[test]
#[ignore = "Set NABI_TEST_GIF to the user-supplied local GIF"]
fn supplied_gif_plays_in_viewer() {
    let source =
        std::path::PathBuf::from(std::env::var_os("NABI_TEST_GIF").expect("NABI_TEST_GIF"));
    let root = crate::test_support::root();
    std::fs::create_dir_all(&root).unwrap();
    let folder = tempfile::tempdir_in(root).unwrap();
    let path = folder.path().join("supplied.gif");
    std::fs::copy(source, &path).unwrap();
    let mut window = TestWindow::open(Some(path));
    let mut previous = Vec::new();
    let mut changes = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while changes < 4 {
        let (text, drawn) = window.draw(None);
        if drawn {
            let texture = window.photo_texture();
            for (_, delta) in window
                .texture_updates
                .iter()
                .filter(|(id, _)| *id == texture)
            {
                if let egui::ImageData::Color(image) = &delta.image {
                    assert_eq!(image.size, [400, 369]);
                    if image.pixels != previous {
                        changes += 1;
                        previous.clone_from(&image.pixels);
                    }
                }
            }
        }
        assert!(Instant::now() < deadline, "GIF did not animate: {text}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn transparency_backdrop_stays_behind_image_without_changing_alpha() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("alpha.png");
    let source = image::RgbaImage::from_fn(96, 64, |x, _| {
        image::Rgba([
            220,
            80,
            40,
            if x < 32 {
                0
            } else if x < 64 {
                128
            } else {
                255
            },
        ])
    });
    source.save(&path).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut window = TestWindow::open(Some(path.clone()));
    window.wait_for("");
    let photo = window.photo_texture();
    let delta = window
        .texture_updates
        .iter()
        .find(|(id, _)| *id == photo)
        .unwrap();
    let egui::ImageData::Color(pixels) = &delta.1.image else {
        panic!("Expected color pixels");
    };
    assert_eq!(pixels.pixels[0].a(), 0);
    assert_eq!(pixels.pixels[40].a(), 128);
    assert_eq!(pixels.pixels[80].a(), 255);
    for _ in 0..2 {
        let (clip, photo_rect) = window.photo_rects();
        let photo_index = window
            .paint
            .iter()
            .position(|shape| {
                matches!(&shape.shape,
            Shape::Mesh(mesh) if mesh.texture_id == photo)
            })
            .unwrap();
        let checker_index = window
            .paint
            .iter()
            .position(|shape| {
                matches!(&shape.shape,
            Shape::Mesh(mesh) if crate::ui::transparency::is_backdrop(&window.ctx, mesh.texture_id))
            })
            .unwrap();
        assert!(checker_index < photo_index);
        let Shape::Mesh(checker) = &window.paint[checker_index].shape else {
            unreachable!()
        };
        assert_eq!(checker.calc_bounds(), photo_rect.intersect(clip));
        assert_eq!(
            checker.vertices.len(),
            4,
            "Checker work must not grow with zoom"
        );
        window.reveal_controls();
        window.click(window.label_position("+"));
    }
    assert_eq!(std::fs::read(path).unwrap(), original);
}
