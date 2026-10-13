//! UVC extension-unit access: request helpers, read-only wrapper, bounded read-only scan and
//! the production device opener (GitHub #378, PR 1 of #353; matrix IRA5–IRA8).

#![cfg(feature = "mock-camera")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_camera_v4l::ir_mock::MockXuControl;
use soos_camera_v4l::uvc_xu::{
    probe_xu_controls, xu_get_cur, xu_get_info, xu_get_len, xu_set_cur, UvcXuDevice, XuBudget,
    XuControl, XuError, XuQuery, XuReadOnly, MAX_PROBE_CONTROL_ERRORS, MAX_PROBE_QUERIES,
    PROBE_MAX_SELECTOR, PROBE_MAX_UNIT_ID, XU_MAX_PAYLOAD_BYTES,
};

fn emitter_mock() -> MockXuControl {
    MockXuControl::new()
        .with_control(14, 6, 0x03, vec![1, 3, 0, 0, 0, 0, 0, 0, 0])
        .with_control(14, 2, 0x01, vec![7, 7])
        .with_control(4, 1, 0x03, vec![0; 70])
}

/// IRA5: request codes are the UVC 1.5 values; only SET_CUR writes.
#[test]
fn test_ira_request_codes_match_uvc_spec() {
    let expected = [
        (XuQuery::SetCur, 0x01, true),
        (XuQuery::GetCur, 0x81, false),
        (XuQuery::GetMin, 0x82, false),
        (XuQuery::GetMax, 0x83, false),
        (XuQuery::GetRes, 0x84, false),
        (XuQuery::GetLen, 0x85, false),
        (XuQuery::GetInfo, 0x86, false),
        (XuQuery::GetDef, 0x87, false),
    ];
    for (query, code, write) in expected {
        assert_eq!(query.code(), code, "{query:?}");
        assert_eq!(query.is_write(), write, "{query:?}");
    }
    assert_eq!(XU_MAX_PAYLOAD_BYTES, 64);
}

/// IRA5: GET_LEN (2 bytes, little endian), GET_INFO, GET_CUR and SET_CUR helpers.
#[test]
fn test_ira_helpers_round_trip_on_mock() {
    let mut mock = emitter_mock();
    assert_eq!(xu_get_len(&mut mock, 14, 6).unwrap(), 9);
    let info = xu_get_info(&mut mock, 14, 6).unwrap();
    assert!(info.supports_get() && info.supports_set());
    let ro = xu_get_info(&mut mock, 14, 2).unwrap();
    assert!(ro.supports_get() && !ro.supports_set());
    assert_eq!(
        xu_get_cur(&mut mock, 14, 6, 9).unwrap(),
        vec![1, 3, 0, 0, 0, 0, 0, 0, 0]
    );
    xu_set_cur(&mut mock, 14, 6, &[1, 3, 3, 0, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(mock.value(14, 6).unwrap(), vec![1, 3, 3, 0, 0, 0, 0, 0, 0]);
    assert_eq!(xu_get_len(&mut mock, 99, 1).unwrap_err(), XuError::NotFound);
}

/// IRA5: zero-length and oversized buffers are refused before any request is issued.
#[test]
fn test_ira_invalid_lengths_are_refused_without_a_request() {
    let mut mock = emitter_mock();
    assert_eq!(
        xu_get_cur(&mut mock, 14, 6, 0).unwrap_err(),
        XuError::InvalidLength { len: 0 }
    );
    assert_eq!(
        xu_get_cur(&mut mock, 14, 6, 65).unwrap_err(),
        XuError::InvalidLength { len: 65 }
    );
    assert_eq!(
        xu_set_cur(&mut mock, 14, 6, &[]).unwrap_err(),
        XuError::InvalidLength { len: 0 }
    );
    assert_eq!(
        xu_set_cur(&mut mock, 14, 6, &[0; 65]).unwrap_err(),
        XuError::InvalidLength { len: 65 }
    );
    assert!(mock.requests().is_empty(), "{:?}", mock.requests());
}

/// IRA6: the read-only wrapper refuses SET_CUR and never forwards it.
#[test]
fn test_ira_read_only_wrapper_never_forwards_a_write() {
    let mut mock = emitter_mock();
    {
        let mut ro = XuReadOnly::new(&mut mock);
        assert_eq!(xu_get_len(&mut ro, 14, 6).unwrap(), 9);
        let mut buf = [1u8; 9];
        assert_eq!(
            ro.query(14, 6, XuQuery::SetCur, &mut buf).unwrap_err(),
            XuError::WriteRefused
        );
        assert_eq!(
            xu_set_cur(&mut ro, 14, 6, &[9; 9]).unwrap_err(),
            XuError::WriteRefused
        );
    }
    assert!(mock.requests().iter().all(|(_, _, q)| !q.is_write()));
    assert_eq!(mock.value(14, 6).unwrap(), vec![1, 3, 0, 0, 0, 0, 0, 0, 0]);
}

/// IRA7: the probe lists the controls, reads values only when GET is supported and the length
/// is within bounds, and issues read requests only.
#[test]
fn test_ira_probe_lists_controls_read_only() {
    let mut mock = emitter_mock();
    let report = probe_xu_controls(&mut mock, 1..=PROBE_MAX_UNIT_ID);
    let found: Vec<(u8, u8, u16)> = report
        .controls
        .iter()
        .map(|c| (c.unit, c.selector, c.len))
        .collect();
    assert_eq!(found, vec![(4, 1, 70), (14, 2, 2), (14, 6, 9)]);
    let emitter = report
        .controls
        .iter()
        .find(|c| c.unit == 14 && c.selector == 6)
        .unwrap();
    assert_eq!(
        emitter.current.as_deref(),
        Some(&[1, 3, 0, 0, 0, 0, 0, 0, 0][..])
    );
    let too_long = report.controls.iter().find(|c| c.unit == 4).unwrap();
    assert_eq!(too_long.current, None, "a 70-byte control is never read");
    assert!(!report.truncated);
    assert!(report.aborted.is_none());
    assert!(mock.requests().iter().all(|(_, _, q)| !q.is_write()));
    assert_eq!(report.queries_issued, mock.requests().len());
    assert!(report.queries_issued <= MAX_PROBE_QUERIES);
    // Every unit of the range is scanned over every selector.
    let len_requests = mock
        .requests()
        .iter()
        .filter(|(_, _, q)| *q == XuQuery::GetLen)
        .count();
    assert_eq!(
        len_requests,
        usize::from(PROBE_MAX_UNIT_ID) * usize::from(PROBE_MAX_SELECTOR)
    );
}

/// IRA7: a narrowed unit range scans that unit only.
#[test]
fn test_ira_probe_single_unit() {
    let mut mock = emitter_mock();
    let report = probe_xu_controls(&mut mock, 14..=14);
    assert_eq!(report.controls.len(), 2);
    assert!(mock.requests().iter().all(|(u, _, _)| *u == 14));
    assert!(mock
        .requests()
        .iter()
        .all(|(_, s, _)| (1..=PROBE_MAX_SELECTOR).contains(s)));
}

/// IRA7: the scan stops at the query budget even over the whole unit space.
#[test]
fn test_ira_probe_is_bounded_by_query_budget() {
    let mut mock = MockXuControl::new();
    for unit in 1..=255u8 {
        for selector in 1..=PROBE_MAX_SELECTOR {
            mock = mock.with_control(unit, selector, 0x03, vec![1]);
        }
    }
    let report = probe_xu_controls(&mut mock, 1..=255);
    assert!(report.truncated);
    assert!(report.queries_issued <= MAX_PROBE_QUERIES);
    assert_eq!(report.queries_issued, mock.requests().len());
}

/// IRA7: a permission or device error aborts the scan and is reported.
#[test]
fn test_ira_probe_aborts_on_permission_error() {
    let mut mock = emitter_mock();
    mock.fail_query(XuQuery::GetLen, XuError::PermissionDenied);
    let report = probe_xu_controls(&mut mock, 1..=PROBE_MAX_UNIT_ID);
    assert_eq!(report.aborted, Some(XuError::PermissionDenied));
    assert!(report.controls.is_empty());
    assert_eq!(
        mock.requests().len(),
        1,
        "the scan stops at the first such error"
    );
}

/// IRA7: a transient error on one control skips it; too many such errors abort the scan.
#[test]
fn test_ira_probe_skips_transient_errors_with_a_bound() {
    let mut mock = emitter_mock();
    mock.fail_query(XuQuery::GetInfo, XuError::Io(5));
    let report = probe_xu_controls(&mut mock, 14..=14);
    assert!(
        report.controls.is_empty(),
        "both controls of unit 14 failed GET_INFO"
    );
    assert!(report.aborted.is_none(), "two errors stay under the bound");

    let mut mock = MockXuControl::new();
    for selector in 1..=PROBE_MAX_SELECTOR {
        mock = mock.with_control(3, selector, 0x03, vec![1]);
    }
    mock.fail_query(XuQuery::GetInfo, XuError::Busy);
    let report = probe_xu_controls(&mut mock, 3..=3);
    assert_eq!(report.aborted, Some(XuError::Busy));
    let info_requests = mock
        .requests()
        .iter()
        .filter(|(_, _, q)| *q == XuQuery::GetInfo)
        .count();
    assert_eq!(info_requests, MAX_PROBE_CONTROL_ERRORS);
}

/// IRA6b (candid review of #378): the request budget refuses the request after the bound
/// without forwarding it, and counts every request it forwarded.
#[test]
fn test_ira_budget_refuses_requests_beyond_the_bound() {
    let mut mock = emitter_mock();
    {
        let mut budget = XuBudget::new(&mut mock, 3);
        assert_eq!(xu_get_len(&mut budget, 14, 6).unwrap(), 9);
        assert!(xu_get_info(&mut budget, 14, 6).is_ok());
        assert_eq!(
            xu_get_len(&mut budget, 99, 1).unwrap_err(),
            XuError::NotFound
        );
        assert_eq!(budget.issued(), 3);
        assert_eq!(
            xu_set_cur(&mut budget, 14, 6, &[0; 9]).unwrap_err(),
            XuError::QueryBudgetExhausted
        );
        assert_eq!(budget.issued(), 3);
    }
    assert_eq!(
        mock.requests().len(),
        3,
        "the refused request never reached the device"
    );
    assert_eq!(mock.value(14, 6).unwrap(), vec![1, 3, 0, 0, 0, 0, 0, 0, 0]);
}

/// IRA8: the production opener refuses anything but a V4L2 character device.
#[test]
fn test_ira_device_open_refuses_non_v4l2_nodes() {
    assert_eq!(
        UvcXuDevice::open(std::path::Path::new("/dev/null")).err(),
        Some(XuError::NotV4l2Device)
    );
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("video0");
    std::fs::write(&file, b"").unwrap();
    assert_eq!(UvcXuDevice::open(&file).err(), Some(XuError::NotV4l2Device));
    assert_eq!(
        UvcXuDevice::open(&dir.path().join("missing")).err(),
        Some(XuError::NotFound)
    );
    assert!(UvcXuDevice::open(std::path::Path::new("/dev/vid\0eo0")).is_err());
}
