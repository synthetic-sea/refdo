use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use super::support::*;
use super::*;

fn themes() -> (Theme, Theme) {
    let light = test_theme();
    let mut dark = light;
    dark.background = Color::Black;
    (light, dark)
}

#[test]
fn system_theme_changes_from_dark_to_light() {
    let (light, dark) = themes();
    let started = Instant::now();
    let mut app =
        App::new_system_with_detector(light, dark, DispatchController::default(), started, || {
            Some(dark_light::Mode::Dark)
        });

    assert_eq!(app.theme, dark);
    app.refresh_system_theme_with(started + Duration::from_millis(500), || {
        Some(dark_light::Mode::Light)
    });

    assert_eq!(app.theme, light);
}

#[test]
fn system_theme_changes_from_light_to_dark() {
    let (light, dark) = themes();
    let started = Instant::now();
    let mut app =
        App::new_system_with_detector(light, dark, DispatchController::default(), started, || {
            Some(dark_light::Mode::Light)
        });

    assert_eq!(app.theme, light);
    app.refresh_system_theme_with(started + Duration::from_millis(500), || {
        Some(dark_light::Mode::Dark)
    });

    assert_eq!(app.theme, dark);
}

#[test]
fn unspecified_and_failed_runtime_detection_preserve_current_theme() {
    let (light, dark) = themes();
    let started = Instant::now();
    let mut app =
        App::new_system_with_detector(light, dark, DispatchController::default(), started, || {
            Some(dark_light::Mode::Dark)
        });

    app.refresh_system_theme_with(started + Duration::from_millis(500), || {
        Some(dark_light::Mode::Unspecified)
    });
    assert_eq!(app.theme, dark);

    app.refresh_system_theme_with(started + Duration::from_millis(1_000), || None);
    assert_eq!(app.theme, dark);
}

#[test]
fn unspecified_and_failed_initial_detection_start_light() {
    let (light, dark) = themes();
    let started = Instant::now();

    let unspecified =
        App::new_system_with_detector(light, dark, DispatchController::default(), started, || {
            Some(dark_light::Mode::Unspecified)
        });
    let failed =
        App::new_system_with_detector(light, dark, DispatchController::default(), started, || None);

    assert_eq!(unspecified.theme, light);
    assert_eq!(failed.theme, light);
}

#[test]
fn fixed_theme_never_detects_or_changes() {
    let (light, _) = themes();
    let mut app = App::new(light, DispatchController::default());
    let detections = Cell::new(0);

    app.refresh_system_theme_with(Instant::now() + Duration::from_secs(1), || {
        detections.set(detections.get() + 1);
        Some(dark_light::Mode::Dark)
    });

    assert_eq!(detections.get(), 0);
    assert_eq!(app.theme, light);
}

#[test]
fn system_detection_is_gated_by_the_refresh_interval() {
    let (light, dark) = themes();
    let started = Instant::now();
    let mut app =
        App::new_system_with_detector(light, dark, DispatchController::default(), started, || {
            Some(dark_light::Mode::Light)
        });
    let detections = Cell::new(0);

    app.refresh_system_theme_with(started + Duration::from_millis(499), || {
        detections.set(detections.get() + 1);
        Some(dark_light::Mode::Dark)
    });
    assert_eq!(detections.get(), 0);
    assert_eq!(app.theme, light);

    app.refresh_system_theme_with(started + Duration::from_millis(500), || {
        detections.set(detections.get() + 1);
        Some(dark_light::Mode::Dark)
    });
    assert_eq!(detections.get(), 1);
    assert_eq!(app.theme, dark);

    app.refresh_system_theme_with(started + Duration::from_millis(999), || {
        detections.set(detections.get() + 1);
        Some(dark_light::Mode::Light)
    });
    assert_eq!(detections.get(), 1);
    assert_eq!(app.theme, dark);

    app.refresh_system_theme_with(started + Duration::from_millis(1_000), || {
        detections.set(detections.get() + 1);
        Some(dark_light::Mode::Light)
    });
    assert_eq!(detections.get(), 2);
    assert_eq!(app.theme, light);
}
