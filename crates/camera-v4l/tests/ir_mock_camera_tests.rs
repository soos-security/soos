//! The mock camera in IR mode: frames follow the mock emitter (GitHub #378, PR 1 of #353;
//! matrix IRA11).

#![cfg(feature = "mock-camera")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use common::{wait_until, SETTLE_TIMEOUT};
use soos_camera_v4l::ir_emitter::{EmitterState, IrEmitter};
use soos_camera_v4l::ir_mock::{
    synthetic_ir_frame, MockEmitterBehavior, MockIrEmitter, MOCK_IR_LIT_LUMA, MOCK_IR_UNLIT_LUMA,
};
use soos_camera_v4l::strobe::{frame_mean_luma, STROBE_DARK_LUMA, STROBE_MIN_LUMA_DELTA};
use soos_camera_v4l::{
    CameraConfigBuilder, CameraManager, MockCameraManager, PixelFormat, SensorType,
};
use std::time::Duration;

/// Lit frames are brighter than unlit ones (checked at compile time).
const _: () = assert!(MOCK_IR_LIT_LUMA > MOCK_IR_UNLIT_LUMA);

fn config() -> soos_camera_v4l::CameraConfig {
    CameraConfigBuilder::new()
        .resolution(64, 48)
        .format(PixelFormat::Yuyv)
        .warmup_frames(0)
        .idle_timeout(Duration::ZERO)
        .fps(30)
        .build()
}

/// IRA11: synthetic IR frames are Grey, Infrared, exact size, and lit/unlit are separated by
/// far more than the classifier gap; unlit frames are dark.
#[test]
fn test_ira_synthetic_ir_frames_are_lit_or_unlit() {
    let lit = synthetic_ir_frame(64, 48, 4, 99, true);
    let unlit = synthetic_ir_frame(64, 48, 5, 100, false);
    for f in [&lit, &unlit] {
        assert_eq!(f.format, PixelFormat::Grey);
        assert_eq!(f.sensor_type, SensorType::Infrared);
        assert_eq!(f.data.len(), 64 * 48);
    }
    assert_eq!((lit.sequence, lit.timestamp_mono_ns), (4, 99));
    let l = frame_mean_luma(&lit).unwrap();
    let u = frame_mean_luma(&unlit).unwrap();
    assert!(l - u >= 4.0 * STROBE_MIN_LUMA_DELTA, "lit {l} unlit {u}");
    assert!(u < STROBE_DARK_LUMA, "unlit {u}");
}

/// IRA11: the mock camera in IR mode publishes frames that follow the commanded emitter.
#[test]
fn test_ira_mock_camera_frames_follow_the_emitter() {
    let mut emitter = MockIrEmitter::new(MockEmitterBehavior::Controllable);
    let camera = MockCameraManager::new_ir(config(), emitter.scene());
    assert!(wait_until(SETTLE_TIMEOUT, || camera
        .latest_frame()
        .is_some()));
    camera.set_frozen(true);

    emitter.set(EmitterState::On).unwrap();
    let lit = camera.step_frame().unwrap();
    assert_eq!(camera.latest_frame().unwrap().sequence, lit.sequence);
    assert_eq!(lit.format, PixelFormat::Grey);
    assert_eq!(lit.sensor_type, SensorType::Infrared);
    let lit_luma = frame_mean_luma(&lit).unwrap();

    emitter.set(EmitterState::Off).unwrap();
    let unlit = camera.step_frame().unwrap();
    assert!(unlit.sequence > lit.sequence);
    let unlit_luma = frame_mean_luma(&unlit).unwrap();
    assert!(
        lit_luma - unlit_luma >= STROBE_MIN_LUMA_DELTA,
        "lit {lit_luma} unlit {unlit_luma}"
    );
    camera.stop();
}

/// IRA11: `step_frame` works while frozen and the wake frame of `notify_activity` also follows
/// the scene.
#[test]
fn test_ira_step_frame_is_deterministic_while_frozen() {
    let mut emitter = MockIrEmitter::new(MockEmitterBehavior::Controllable);
    let camera = MockCameraManager::new_ir(config(), emitter.scene());
    camera.set_frozen(true);
    let mut lumas = Vec::new();
    for on in [true, false, true, true, false] {
        emitter
            .set(if on {
                EmitterState::On
            } else {
                EmitterState::Off
            })
            .unwrap();
        let frame = camera.step_frame().unwrap();
        lumas.push((on, frame_mean_luma(&frame).unwrap()));
    }
    for (on, luma) in lumas {
        assert_eq!(luma >= STROBE_DARK_LUMA, on, "luma {luma} for on = {on}");
    }
    camera.set_frozen(false);
    assert!(wait_until(SETTLE_TIMEOUT, || camera.is_ready()));
    emitter.set(EmitterState::On).unwrap();
    camera.notify_activity();
    let wake = camera.latest_frame().unwrap();
    assert_eq!(wake.sensor_type, SensorType::Infrared);
    camera.stop();
}

/// IRA11: the regular constructor is unchanged (frames keep the configured format).
#[test]
fn test_ira_plain_mock_camera_is_unchanged() {
    let camera = MockCameraManager::new(config());
    assert!(wait_until(SETTLE_TIMEOUT, || camera
        .latest_frame()
        .is_some()));
    let frame = camera.latest_frame().unwrap();
    assert_eq!(frame.format, PixelFormat::Yuyv);
    assert_eq!(frame.sensor_type, SensorType::Unknown);
    camera.stop();
}
