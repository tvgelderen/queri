use gpui_kit::{BackdropBlur, Bounds, ContentMask, ScaledPixels, Scene, point, size};

fn blur(x: f32) -> BackdropBlur {
    let bounds = Bounds {
        origin: point(ScaledPixels(x), ScaledPixels(0.)),
        size: size(ScaledPixels(100.), ScaledPixels(100.)),
    };
    BackdropBlur {
        order: 0,
        bounds,
        content_mask: ContentMask { bounds },
        blur_radius: ScaledPixels(8.),
        opacity: 1.,
    }
}

#[test]
fn consecutive_and_trailing_blurs_keep_separate_sampling_boundaries() {
    let mut scene = Scene::default();
    scene.insert_backdrop_blur(blur(0.));
    scene.insert_backdrop_blur(blur(0.));
    scene.finish();
    let orders: Vec<_> = scene
        .batches()
        .map(|batch| scene.batch_first_order(&batch))
        .collect();
    assert_eq!(orders.len(), 2);
    assert!(orders[0] < orders[1]);
    assert_eq!(orders[0], scene.backdrop_blurs[0].order);
    assert_eq!(orders[1], scene.backdrop_blurs[1].order);
}

#[test]
fn cached_scene_replay_preserves_blurs_without_duplicating_markers() {
    let mut original = Scene::default();
    original.insert_backdrop_blur(blur(0.));
    original.insert_backdrop_blur(blur(0.));
    original.finish();
    let mut replay = Scene::default();
    replay.replay(0..original.len(), &original);
    replay.finish();
    assert_eq!(replay.backdrop_blurs.len(), 2);
    assert_eq!(replay.shadows.len(), 2);
    assert_eq!(replay.batches().count(), 2);
    replay.clear();
    assert!(replay.backdrop_blurs.is_empty());
    assert!(replay.shadows.is_empty());
    assert_eq!(replay.len(), 0);
}

#[test]
fn fully_clipped_blur_does_not_allocate_a_sampling_barrier() {
    let mut clipped = blur(0.);
    clipped.content_mask = blur(200.).content_mask;
    let mut scene = Scene::default();
    scene.insert_backdrop_blur(clipped);
    assert!(scene.backdrop_blurs.is_empty());
    assert!(scene.shadows.is_empty());
    assert_eq!(scene.len(), 0);
}
