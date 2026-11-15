use super::*;

#[test]
fn gif_reuses_texture_and_preserves_frames_across_fullscreen_resize() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("animated-alpha.gif");
    let colors = [Color32::RED, Color32::GREEN, Color32::BLUE, Color32::YELLOW];
    let expected: Vec<Vec<Color32>> = colors
        .iter()
        .map(|&color| {
            (0..48 * 32)
                .map(|pixel| {
                    if pixel % 48 < 8 {
                        Color32::TRANSPARENT
                    } else {
                        color
                    }
                })
                .collect()
        })
        .collect();
    {
        let mut encoder =
            image::codecs::gif::GifEncoder::new(std::fs::File::create(&path).unwrap());
        for color in colors {
            let image = image::RgbaImage::from_fn(48, 32, |x, _| {
                image::Rgba(if x < 8 { [0; 4] } else { color.to_array() })
            });
            encoder
                .encode_frame(image::Frame::from_parts(
                    image,
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(40, 1),
                ))
                .unwrap();
        }
    }
    let original = std::fs::read(&path).unwrap();
    let mut window = TestWindow::open(Some(path.clone()));
    let mut texture = None;
    let mut previous = None;
    let mut changes = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    for stage in 0..3 {
        window.screen_size = if stage == 1 {
            egui::vec2(1920., 1080.)
        } else {
            egui::vec2(900., 600.)
        };
        let until = changes + 3;
        let mut first_pass = true;
        while changes < until {
            let key = (stage > 0 && first_pass).then_some(Key::F);
            let (text, drawn) = window.draw(key);
            if key.is_some() {
                assert!(
                    window
                        .viewport_commands
                        .contains(&egui::ViewportCommand::Fullscreen(stage == 1))
                );
            }
            first_pass = false;
            assert!(!text.contains("ERROR"), "{text}");
            if drawn {
                let photo = window.photo_texture();
                if let Some(texture) = texture {
                    assert_eq!(photo, texture, "Resizing must preserve the playing texture");
                }
                for (_, delta) in window.texture_updates.iter().filter(|(id, _)| *id == photo) {
                    let egui::ImageData::Color(image) = &delta.image;
                    assert_eq!(image.size, [48, 32]);
                    let index = expected
                        .iter()
                        .position(|pixels| *pixels == image.pixels)
                        .expect(
                            "Composited frame pixels and transparent columns must be preserved",
                        );
                    if texture.is_none() {
                        assert_eq!(delta.pos, None, "The first frame allocates the texture");
                        assert_eq!(index, 0);
                        texture = Some(photo);
                    } else {
                        assert_eq!(
                            delta.pos,
                            Some([0, 0]),
                            "Later frames must update the existing GPU texture"
                        );
                    }
                    if let Some(previous) = previous
                        && previous != index
                    {
                        assert_eq!(index, (previous + 1) % expected.len());
                        changes += 1;
                    }
                    previous = Some(index);
                }
            }
            assert!(
                Instant::now() < deadline,
                "Animation stalled at stage {stage}: {text}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert_eq!(std::fs::read(path).unwrap(), original);
}

/// Measure the actual Viewer texture updates, independently of the test UI's
/// synthetic input clock. Frequent redraws must not accelerate playback.
fn assert_viewer_animation_timing(path: std::path::PathBuf, delays: &[Duration], cycles: usize) {
    assert!(delays.len() > 1);
    let expected_cycle: Duration = delays.iter().sum();
    let expected_total = expected_cycle * cycles as u32;
    let mut window = TestWindow::open(Some(path));
    let deadline = Instant::now() + expected_total + Duration::from_secs(20);
    let mut first = None;
    let mut previous = None;
    let mut updates = 0;
    let mut elapsed_expected = Duration::ZERO;
    let mut draws = 0;
    loop {
        let before_draw = Instant::now();
        let (text, drawn) = window.draw(None);
        let after_draw = Instant::now();
        draws += 1;
        assert!(!text.contains("ERROR"), "{text}");
        if drawn {
            let texture = window.photo_texture();
            for (_, delta) in window
                .texture_updates
                .iter()
                .filter(|(id, delta)| *id == texture && delta.pos == Some([0, 0]))
            {
                // Loading supplies a full first-frame texture. Its first partial
                // update is frame zero from the playback worker, so loading and
                // worker startup are excluded from the measured cycle.
                let egui::ImageData::Color(image) = &delta.image;
                assert!(!image.pixels.is_empty());
                let first = *first.get_or_insert(before_draw);
                if let Some(previous) = previous {
                    let delay = delays[(updates - 1) % delays.len()];
                    elapsed_expected += delay;
                    // Bound the observation uncertainty by the two UI calls,
                    // rather than accepting an arbitrary early-frame tolerance.
                    assert!(
                        after_draw.duration_since(previous) >= delay,
                        "Frame {updates} advanced before its {delay:?} delay elapsed"
                    );
                    assert!(
                        after_draw.duration_since(first) >= elapsed_expected,
                        "Playback advanced faster than the expected timeline"
                    );
                }
                if updates == delays.len() * cycles {
                    println!(
                        "{} frames x {cycles} cycles: expected {expected_total:?}, observed {:?}, {draws} UI redraws",
                        delays.len(),
                        after_draw.duration_since(first)
                    );
                    return;
                }
                previous = Some(before_draw);
                updates += 1;
            }
        }
        assert!(
            Instant::now() < deadline,
            "Animation stalled after {updates} texture updates: {text}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn repeated_redraws_preserve_gif_webp_apng_and_avif_playback_speed() {
    use crate::core::media::animation_fixtures;

    let folder = tempfile::tempdir().unwrap();
    let gif = folder.path().join("timing.gif");
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(std::fs::File::create(&gif).unwrap());
        for (color, delay) in [
            (Color32::RED, 60),
            (Color32::GREEN, 70),
            (Color32::BLUE, 120),
        ] {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(16, 16, image::Rgba(color.to_array())),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(delay, 1),
                ))
                .unwrap();
        }
    }
    assert_viewer_animation_timing(gif, &[60, 70, 120].map(Duration::from_millis), 2);

    let short_gif = folder.path().join("short-delay.gif");
    {
        let mut encoder =
            image::codecs::gif::GifEncoder::new(std::fs::File::create(&short_gif).unwrap());
        for (color, delay) in [(Color32::RED, 0), (Color32::GREEN, 10), (Color32::BLUE, 20)] {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(16, 16, image::Rgba(color.to_array())),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(delay, 1),
                ))
                .unwrap();
        }
    }
    assert_viewer_animation_timing(short_gif, &[100, 100, 20].map(Duration::from_millis), 2);

    let webp = folder.path().join("timing.webp");
    animation_fixtures::webp(&webp);
    assert_viewer_animation_timing(webp, &[100, 200].map(Duration::from_millis), 2);

    let apng = folder.path().join("timing.apng");
    animation_fixtures::apng(&apng);
    assert_viewer_animation_timing(apng, &[100, 200].map(Duration::from_millis), 2);

    let avif = folder.path().join("timing.avif");
    std::fs::write(&avif, animation_fixtures::avif_bytes(16, 16, 2)).unwrap();
    assert_viewer_animation_timing(avif, &[Duration::from_millis(140); 2], 2);
}

#[test]
#[ignore = "Set NABI_TEST_GIF to a local GIF to measure a complete playback cycle"]
fn supplied_gif_plays_in_viewer() {
    use image::AnimationDecoder;

    let source =
        std::path::PathBuf::from(std::env::var_os("NABI_TEST_GIF").expect("NABI_TEST_GIF"));
    let root = crate::test_support::root();
    std::fs::create_dir_all(&root).unwrap();
    let folder = tempfile::tempdir_in(root).unwrap();
    let path = folder.path().join("supplied.gif");
    std::fs::copy(source, &path).unwrap();
    let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
        std::fs::File::open(&path).unwrap(),
    ))
    .unwrap();
    let encoded_delays: Vec<_> = decoder
        .into_frames()
        .map(|frame| {
            let (n, d) = frame.unwrap().delay().numer_denom_ms();
            Duration::from_secs_f64(n as f64 / d as f64 / 1000.)
        })
        .collect();
    let delays: Vec<_> = encoded_delays
        .iter()
        .map(|&delay| {
            if delay <= Duration::from_millis(10) {
                Duration::from_millis(100)
            } else {
                delay
            }
        })
        .collect();
    println!(
        "Encoded cycle {:?}; expected browser-compatible playback {:?}",
        encoded_delays.iter().sum::<Duration>(),
        delays.iter().sum::<Duration>()
    );
    assert_viewer_animation_timing(path, &delays, 1);
}

#[test]
fn transparency_backdrop_is_canvas_only_without_changing_alpha() {
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
    let egui::ImageData::Color(pixels) = &delta.1.image;
    assert_eq!(pixels.pixels[0].a(), 0);
    assert_eq!(pixels.pixels[40].a(), 128);
    assert_eq!(pixels.pixels[80].a(), 255);
    for controls_visible in [false, true] {
        let (clip, photo_rect) = window.photo_rects();
        let photo_index = window
            .paint
            .iter()
            .position(|shape| {
                matches!(&shape.shape,
            Shape::Mesh(mesh) if mesh.texture_id == photo)
            })
            .unwrap();
        let checkers: Vec<_> = window
            .paint
            .iter()
            .enumerate()
            .filter(|(_, shape)| {
                matches!(&shape.shape,
            Shape::Mesh(mesh) if crate::ui::transparency::is_backdrop(&window.ctx, mesh.texture_id))
            })
            .collect();
        assert_eq!(checkers.len(), 1, "Only the canvas gets a checkerboard");
        let checker_index = checkers[0].0;
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
        if controls_visible {
            assert!(
                window.paint.iter().any(|shape| {
                    matches!(&shape.shape, Shape::Mesh(mesh)
                        if mesh.texture_id == photo && shape.clip_rect.height() == 57.)
                }),
                "The thumbnail must reuse the image texture with its original alpha"
            );
        }
        window.reveal_controls();
        window.click(window.label_position("+"));
    }
    assert_eq!(std::fs::read(path).unwrap(), original);
}
