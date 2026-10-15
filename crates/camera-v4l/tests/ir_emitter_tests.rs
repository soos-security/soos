//! `IrEmitter`: the UVC extension-unit emitter (restore on every path) and the mock emitter
//! that drives the mock camera (GitHub #378, PR 1 of #353; matrix IRA9–IRA11).

#![cfg(feature = "mock-camera")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_camera_v4l::ir_emitter::{EmitterState, IrEmitter, IrEmitterError, UvcXuEmitter};
use soos_camera_v4l::ir_emitter_config::{UsbDeviceId, XuEmitterDescriptor};
use soos_camera_v4l::ir_mock::{MockEmitterBehavior, MockIrEmitter, MockXuControl};
use soos_camera_v4l::uvc_xu::{XuError, XuQuery};

const ORIGINAL: [u8; 3] = [9, 9, 9];

fn descriptor() -> XuEmitterDescriptor {
    XuEmitterDescriptor::new(
        UsbDeviceId {
            vendor: 0x04f2,
            product: 0xb6d9,
        },
        14,
        6,
        vec![1, 3, 3],
        vec![1, 3, 0],
    )
    .unwrap()
}

fn mock(info: u8, len: usize) -> MockXuControl {
    let mut value = ORIGINAL.to_vec();
    value.resize(len, 9);
    MockXuControl::new().with_control(14, 6, info, value)
}

fn sets(mock: &MockXuControl) -> usize {
    mock.requests()
        .iter()
        .filter(|(_, _, q)| *q == XuQuery::SetCur)
        .count()
}

/// IRA9: opening checks GET_INFO and GET_LEN and reads the original value, writing nothing.
#[test]
fn test_ira_open_reads_capability_and_writes_nothing() {
    let mut control = mock(0x03, 3);
    {
        let mut emitter = UvcXuEmitter::open(&mut control, descriptor()).unwrap();
        let cap = emitter.capability().unwrap();
        assert!(cap.can_read && cap.can_write);
        assert_eq!(cap.payload_len, 3);
        assert_eq!(emitter.commanded(), None);
        assert_eq!(emitter.descriptor(), &descriptor());
    }
    assert_eq!(sets(&control), 0, "open and drop without set never write");
    assert_eq!(control.value(14, 6).unwrap(), ORIGINAL.to_vec());
}

/// IRA9: a control of another length, or one that cannot be written, is refused at open.
#[test]
fn test_ira_open_refuses_mismatched_or_read_only_control() {
    let mut control = mock(0x03, 4);
    assert_eq!(
        UvcXuEmitter::open(&mut control, descriptor()).err(),
        Some(IrEmitterError::PayloadLengthMismatch {
            control: 4,
            configured: 3
        })
    );
    let mut control = mock(0x01, 3);
    assert_eq!(
        UvcXuEmitter::open(&mut control, descriptor()).err(),
        Some(IrEmitterError::NotWritable)
    );
    let mut control = MockXuControl::new();
    assert_eq!(
        UvcXuEmitter::open(&mut control, descriptor()).err(),
        Some(IrEmitterError::Control(XuError::NotFound))
    );
    assert_eq!(sets(&control), 0);
}

/// IRA10: set writes the configured payloads; restore and drop put the original back.
#[test]
fn test_ira_set_writes_payloads_and_drop_restores_original() {
    let mut control = mock(0x03, 3);
    {
        let mut emitter = UvcXuEmitter::open(&mut control, descriptor()).unwrap();
        emitter.set(EmitterState::On).unwrap();
        assert_eq!(emitter.commanded(), Some(EmitterState::On));
        emitter.set(EmitterState::Off).unwrap();
        assert_eq!(emitter.commanded(), Some(EmitterState::Off));
    }
    assert_eq!(control.value(14, 6).unwrap(), ORIGINAL.to_vec());
    let writes: Vec<_> = control
        .requests()
        .iter()
        .filter(|(_, _, q)| *q == XuQuery::SetCur)
        .collect();
    assert_eq!(writes.len(), 3, "on, off, then the restore");
}

/// IRA10: restore is idempotent and drop does not write again after it.
#[test]
fn test_ira_restore_is_idempotent() {
    let mut control = mock(0x03, 3);
    {
        let mut emitter = UvcXuEmitter::open(&mut control, descriptor()).unwrap();
        emitter.set(EmitterState::On).unwrap();
        emitter.restore().unwrap();
        emitter.restore().unwrap();
    }
    assert_eq!(sets(&control), 2);
    assert_eq!(control.value(14, 6).unwrap(), ORIGINAL.to_vec());
}

/// IRA10: a failed set keeps the emitter dirty, so the original is still restored.
#[test]
fn test_ira_failed_set_still_restores() {
    let mut control = mock(0x03, 3);
    control.fail_set_after(1);
    {
        let mut emitter = UvcXuEmitter::open(&mut control, descriptor()).unwrap();
        emitter.set(EmitterState::On).unwrap();
        assert!(emitter.set(EmitterState::Off).is_err());
        assert_eq!(emitter.commanded(), Some(EmitterState::On));
        // The device refuses every further write: restore reports it loudly.
        assert!(matches!(
            emitter.restore(),
            Err(IrEmitterError::RestoreFailed(_))
        ));
    }
    let restores = control
        .requests()
        .iter()
        .filter(|(_, _, q)| *q == XuQuery::SetCur)
        .count();
    assert!(
        restores >= 3,
        "on, failed off, at least one restore attempt"
    );
}

/// IRA11: the mock emitter follows commands only when controllable, and the shared scene
/// tells the mock camera which frames are lit.
#[test]
fn test_ira_mock_emitter_behaviours() {
    let mut controllable = MockIrEmitter::new(MockEmitterBehavior::Controllable);
    let scene = controllable.scene();
    assert!(!scene.is_lit(0), "starts off");
    controllable.set(EmitterState::On).unwrap();
    assert!(scene.is_lit(0) && scene.is_lit(1));
    assert_eq!(controllable.commanded(), Some(EmitterState::On));
    assert!(controllable.capability().unwrap().can_write);
    controllable.set(EmitterState::Off).unwrap();
    assert!(!scene.is_lit(7));
    assert_eq!(controllable.set_calls(), 2);

    let mut native = MockIrEmitter::new(MockEmitterBehavior::NativeAlternating);
    let scene = native.scene();
    native.set(EmitterState::Off).unwrap();
    assert!(scene.is_lit(0) && !scene.is_lit(1) && scene.is_lit(2));
    assert!(!native.capability().unwrap().can_write);

    let mut steady = MockIrEmitter::new(MockEmitterBehavior::Steady);
    steady.set(EmitterState::Off).unwrap();
    assert!(steady.scene().is_lit(3));

    let mut absent = MockIrEmitter::new(MockEmitterBehavior::Absent);
    absent.set(EmitterState::On).unwrap();
    assert!(!absent.scene().is_lit(3));

    let mut failing = MockIrEmitter::new(MockEmitterBehavior::Controllable);
    failing.fail_next_set();
    assert_eq!(
        failing.set(EmitterState::On).unwrap_err(),
        IrEmitterError::Rejected
    );
    assert_eq!(failing.commanded(), None);
    assert!(!failing.scene().is_lit(0));
    failing.set(EmitterState::On).unwrap();
    assert!(failing.scene().is_lit(0));
}

/// IRA9: the trait is object safe (the daemon will hold a `Box<dyn IrEmitter>`).
#[test]
fn test_ira_ir_emitter_is_object_safe() {
    let mut boxed: Box<dyn IrEmitter> =
        Box::new(MockIrEmitter::new(MockEmitterBehavior::Controllable));
    boxed.set(EmitterState::On).unwrap();
    assert_eq!(boxed.commanded(), Some(EmitterState::On));
}

/// IRA10: `get` reads the device: on, off, any other value.
#[test]
fn test_ira_get_reads_device_state() {
    let mut control = mock(0x03, 3);
    let mut emitter = UvcXuEmitter::open(&mut control, descriptor()).unwrap();
    assert_eq!(
        emitter.get().unwrap(),
        None,
        "the original [9, 9, 9] is neither payload"
    );
    emitter.set(EmitterState::On).unwrap();
    assert_eq!(emitter.get().unwrap(), Some(EmitterState::On));
    emitter.set(EmitterState::Off).unwrap();
    assert_eq!(emitter.get().unwrap(), Some(EmitterState::Off));
}

/// IRA10: a failed read is an error, never a guessed state; the original is still restored.
#[test]
fn test_ira_get_failure_is_an_error() {
    let mut control = mock(0x03, 3);
    // open reads GET_CUR once (the original value); the next GET_CUR fails.
    control.fail_get_cur_after(1, XuError::Busy);
    {
        let mut emitter = UvcXuEmitter::open(&mut control, descriptor()).unwrap();
        emitter.set(EmitterState::On).unwrap();
        assert_eq!(
            emitter.get().unwrap_err(),
            IrEmitterError::Control(XuError::Busy)
        );
    }
    assert_eq!(control.value(14, 6).unwrap(), ORIGINAL.to_vec());
}

/// IRA11: the mock emitter reports its scene state through `get`.
#[test]
fn test_ira_mock_get_reports_scene() {
    let mut emitter = MockIrEmitter::new(MockEmitterBehavior::Controllable);
    assert_eq!(emitter.get().unwrap(), Some(EmitterState::Off));
    emitter.set(EmitterState::On).unwrap();
    assert_eq!(emitter.get().unwrap(), Some(EmitterState::On));
    let mut absent = MockIrEmitter::new(MockEmitterBehavior::Absent);
    absent.set(EmitterState::On).unwrap();
    assert_eq!(absent.get().unwrap(), Some(EmitterState::Off));
}
