//! Property-based and fuzzing test suite for `soos-protocol`.
//!
//! Validates:
//! - Serialization / deserialization round-trip for arbitrary valid messages
//! - Codec bounds enforcement (declared size, message boundaries, service length)
//! - Decoder resilience (zero panics on arbitrary, corrupted, and adversarial byte sequences)

#![forbid(unsafe_code)]

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Property tests use assertions, panics, casts, and slicing in test harnesses"
)]
#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use soos_protocol::codec::CodecError;
    use soos_protocol::types::{
        Event, EventKind, ReasonClass, Request, RequestId, RequestKind, Response, StatusResponse,
        ValidationError, Verdict, CURRENT_VERSION, MAX_MESSAGE_SIZE, MAX_SERVICE_LEN,
        REQUEST_ID_LEN,
    };
    use soos_protocol::{decode, encode};

    // ---------------------------------------------------------------------------
    // Strategies for Arbitrary Protocol Data Generation
    // ---------------------------------------------------------------------------

    /// Strategy generating arbitrary 256-bit (32-byte) request IDs.
    fn arb_request_id() -> impl Strategy<Value = RequestId> {
        proptest::collection::vec(any::<u8>(), REQUEST_ID_LEN).prop_map(|v| {
            let mut id = [0u8; REQUEST_ID_LEN];
            id.copy_from_slice(&v);
            id
        })
    }

    /// Strategy generating valid RequestKind variants.
    fn arb_request_kind() -> impl Strategy<Value = RequestKind> {
        prop_oneof![Just(RequestKind::Auth), Just(RequestKind::Status)]
    }

    /// Strategy generating valid EventKind variants.
    fn arb_event_kind() -> impl Strategy<Value = EventKind> {
        Just(EventKind::PasswordFailed)
    }

    /// Strategy generating bounded PAM service names (1 to 64 bytes, printable ASCII).
    fn arb_valid_service_name() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[a-zA-Z0-9._-]{1,64}")
            .expect("valid regex for PAM service name")
    }

    /// Strategy generating all Verdict variants.
    fn arb_verdict() -> impl Strategy<Value = Verdict> {
        prop_oneof![
            Just(Verdict::Allow),
            Just(Verdict::Deny),
            Just(Verdict::Unavailable),
            Just(Verdict::ProtocolError),
        ]
    }

    /// Strategy generating all ReasonClass variants.
    fn arb_reason_class() -> impl Strategy<Value = ReasonClass> {
        prop_oneof![
            Just(ReasonClass::FaceMatch),
            Just(ReasonClass::NoFace),
            Just(ReasonClass::MultipleFaces),
            Just(ReasonClass::ScoreBelowThreshold),
            Just(ReasonClass::PadFailed),
            Just(ReasonClass::CameraUnavailable),
            Just(ReasonClass::ModelUnavailable),
            Just(ReasonClass::StaleFrame),
            Just(ReasonClass::Timeout),
            Just(ReasonClass::RateLimited),
            Just(ReasonClass::UidMismatch),
            Just(ReasonClass::MalformedRequest),
            Just(ReasonClass::InternalError),
        ]
    }

    /// Strategy generating arbitrary valid Request structures.
    fn arb_valid_request() -> impl Strategy<Value = Request> {
        (
            arb_request_kind(),
            arb_request_id(),
            any::<u32>(),
            arb_valid_service_name(),
            any::<u64>(),
        )
            .prop_map(
                |(kind, request_id, uid_hint, service, deadline_monotonic_ns)| Request {
                    version: CURRENT_VERSION,
                    kind,
                    request_id,
                    uid_hint,
                    service,
                    deadline_monotonic_ns,
                },
            )
    }

    /// Strategy generating arbitrary valid Response structures.
    fn arb_valid_response() -> impl Strategy<Value = Response> {
        (
            arb_request_id(),
            arb_verdict(),
            arb_reason_class(),
            any::<u64>(),
            any::<u64>(),
        )
            .prop_map(
                |(request_id, verdict, reason_class, issued_monotonic_ns, expires_monotonic_ns)| {
                    Response {
                        version: CURRENT_VERSION,
                        request_id,
                        verdict,
                        reason_class,
                        issued_monotonic_ns,
                        expires_monotonic_ns,
                    }
                },
            )
    }

    /// Strategy generating arbitrary valid Event structures.
    fn arb_valid_event() -> impl Strategy<Value = Event> {
        (
            arb_event_kind(),
            proptest::option::of(arb_request_id()),
            arb_valid_service_name(),
            any::<u64>(),
        )
            .prop_map(
                |(kind, request_id, service, timestamp_monotonic_ns)| Event {
                    version: CURRENT_VERSION,
                    kind,
                    request_id,
                    service,
                    timestamp_monotonic_ns,
                },
            )
    }

    /// Strategy generating arbitrary valid StatusResponses.
    fn arb_valid_status_response() -> impl Strategy<Value = StatusResponse> {
        (
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
            any::<u32>(),
            any::<u64>(),
        )
            .prop_map(
                |(socket_ready, camera_ready, models_verified, is_healthy, pid, uptime_secs)| {
                    StatusResponse {
                        version: CURRENT_VERSION,
                        socket_ready,
                        camera_ready,
                        models_verified,
                        is_healthy,
                        pid,
                        uptime_secs,
                    }
                },
            )
    }

    // ---------------------------------------------------------------------------
    // Property Tests
    // ---------------------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(250))]

        /// Sub-issue #4.3: Request round-trip property.
        /// Any valid Request serialized via `encode` must deserialize via `decode`
        /// into an identical Request, and must satisfy `validate()`.
        #[test]
        fn prop_request_roundtrip(req in arb_valid_request()) {
            prop_assert!(req.validate().is_ok());

            let encoded = encode(&req).expect("encoding valid request must succeed");
            let decoded: Request = decode(&encoded).expect("decoding valid encoded request must succeed");

            prop_assert_eq!(&decoded, &req);
            prop_assert!(decoded.validate().is_ok());
        }

        /// Response round-trip property.
        /// Any valid Response serialized via `encode` must deserialize via `decode`
        /// into an identical Response.
        #[test]
        fn prop_response_roundtrip(resp in arb_valid_response()) {
            let encoded = encode(&resp).expect("encoding valid response must succeed");
            let decoded: Response = decode(&encoded).expect("decoding valid encoded response must succeed");

            prop_assert_eq!(&decoded, &resp);
        }

        /// Event round-trip property.
        /// Any valid Event serialized via `encode` must deserialize via `decode`
        /// into an identical Event.
        #[test]
        fn prop_event_roundtrip(evt in arb_valid_event()) {
            let encoded = encode(&evt).expect("encoding valid event must succeed");
            let decoded: Event = decode(&encoded).expect("decoding valid encoded event must succeed");

            prop_assert_eq!(&decoded, &evt);
        }

        /// StatusResponse round-trip property.
        /// Any valid StatusResponse serialized via `encode` must deserialize via `decode`
        /// into an identical StatusResponse.
        #[test]
        fn prop_status_response_roundtrip(status_resp in arb_valid_status_response()) {
            let encoded = encode(&status_resp).expect("encoding valid status response must succeed");
            let decoded: StatusResponse = decode(&encoded).expect("decoding valid encoded status response must succeed");

            prop_assert_eq!(&decoded, &status_resp);
        }

        /// Criterion P5 / Sub-issue #4.1: Robustness against arbitrary byte sequences.
        /// Feeding completely random bytes to `decode::<Request>` must NEVER panic.
        /// It must return either Ok(Request) or Err(CodecError).
        #[test]
        fn prop_decode_request_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..=8192)) {
            let res: Result<Request, CodecError> = decode(&bytes);
            match res {
                Ok(req) => {
                    // If decoding succeeded, validate() must also never panic
                    let _ = req.validate();
                }
                Err(_) => {
                    // Rejection is expected and desired for arbitrary random bytes
                }
            }
        }

        /// Criterion P5 / Sub-issue #4.2: Robustness against arbitrary byte sequences.
        /// Feeding completely random bytes to `decode::<Response>` must NEVER panic.
        /// It must return either Ok(Response) or Err(CodecError).
        #[test]
        fn prop_decode_response_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..=8192)) {
            let res: Result<Response, CodecError> = decode(&bytes);
            match res {
                Ok(resp) => {
                    let _ = resp.is_allow();
                }
                Err(_) => {
                    // Rejection is expected and desired for arbitrary random bytes
                }
            }
        }

        /// Decoder robustness for Event against arbitrary byte sequences.
        #[test]
        fn prop_decode_event_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..=8192)) {
            let _ = decode::<Event>(&bytes);
        }

        /// Criterion P2: Any buffer whose declared size prefix exceeds MAX_MESSAGE_SIZE (4096)
        /// MUST be rejected immediately with DeclaredSizeTooLarge, with zero body allocation.
        #[test]
        fn prop_declared_size_bounds(
            declared in (MAX_MESSAGE_SIZE as u32 + 1)..=u32::MAX,
            payload in proptest::collection::vec(any::<u8>(), 0..=128),
        ) {
            let mut buf = Vec::with_capacity(4 + payload.len());
            buf.extend_from_slice(&declared.to_be_bytes());
            buf.extend_from_slice(&payload);

            let res_req: Result<Request, CodecError> = decode(&buf);
            match res_req {
                Err(CodecError::DeclaredSizeTooLarge { declared: d, max }) => {
                    prop_assert_eq!(d, declared as usize);
                    prop_assert_eq!(max, MAX_MESSAGE_SIZE);
                }
                other => {
                    panic!("Expected DeclaredSizeTooLarge, got: {:?}", other);
                }
            }

            let res_resp: Result<Response, CodecError> = decode(&buf);
            match res_resp {
                Err(CodecError::DeclaredSizeTooLarge { declared: d, max }) => {
                    prop_assert_eq!(d, declared as usize);
                    prop_assert_eq!(max, MAX_MESSAGE_SIZE);
                }
                other => {
                    panic!("Expected DeclaredSizeTooLarge, got: {:?}", other);
                }
            }
        }

        /// Truncated buffer property.
        /// Any buffer smaller than 4 bytes, or smaller than 4 + declared_size,
        /// MUST return CodecError::BufferTooSmall.
        #[test]
        fn prop_truncated_buffer_bounds(
            declared in 1u32..=(MAX_MESSAGE_SIZE as u32),
            truncated_len in 0usize..=3usize,
        ) {
            let mut buf = Vec::new();
            buf.extend_from_slice(&declared.to_be_bytes());
            buf.truncate(truncated_len);

            let res: Result<Request, CodecError> = decode(&buf);
            match res {
                Err(CodecError::BufferTooSmall) => {}
                other => panic!("Expected BufferTooSmall, got: {:?}", other),
            }
        }

        /// Oversized service name validation property.
        /// Any Request containing a service name > MAX_SERVICE_LEN (64) bytes
        /// MUST fail validate() with ValidationError::ServiceTooLong.
        #[test]
        fn prop_oversized_service_validation(
            service in proptest::string::string_regex("[a-zA-Z0-9._-]{65,128}").expect("valid regex"),
        ) {
            let req = Request {
                version: CURRENT_VERSION,
                kind: RequestKind::Auth,
                request_id: [0x55; REQUEST_ID_LEN],
                uid_hint: 1000,
                service: service.clone(),
                deadline_monotonic_ns: 100_000_000,
            };

            match req.validate() {
                Err(ValidationError::ServiceTooLong { len, max }) => {
                    prop_assert_eq!(len, service.len());
                    prop_assert_eq!(max, MAX_SERVICE_LEN);
                }
                other => panic!("Expected ServiceTooLong error, got: {:?}", other),
            }
        }
    }
}
