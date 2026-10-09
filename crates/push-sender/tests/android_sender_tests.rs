//! Android coverage contract of the push sender library `soos-push-sender` (GitHub #349, ADR
//! 2026-10-09 "Android Support for the `soos-remote` Phone Companion and Web Push", architect
//! spec `AI/architect_spec_remote_android.md` §11.2; matrix RAN9).
//!
//! The wire already accepts Android endpoints (spec §0.1); this suite pins that behaviour. No
//! network: DNS answers come from an injected lookup and the HTTP side of `serve_connection`
//! is a fake `Deliverer`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use zeroize::Zeroizing;

use soos_push_protocol::{
    decode_reply, encode_request, DeliveryReply, DeliveryRequest, Outcome, PushEndpoint, Urgency,
};
use soos_push_sender::{filter_addresses, serve_connection, AddressLookup, Deliverer, LookupError};

/// The four Android endpoint shapes of spec §11.2.
const ANDROID_ENDPOINTS: [&str; 4] = [
    "https://fcm.googleapis.com/fcm/send/eT4wQx9-R_k:APA91bF0q8mD2nK7-vL3_pZ9sXyWcE5tH1uJ6oR4iA8gB2dN0mQ7kS3fV9xC1zY5wT6rE2_uI8oP4aL0jH7gD3sK9nM5bV1cX6zQ2wE8rT4yU0iO7pA3sD",
    "https://fcm.googleapis.com/wp/fXy7Kd2pQ1s:APA91bE8rT4yU0iO7pA3sD9fG2hJ5kL1zX6cV8bN3mQ0wE4rT7yU2iO5pA9sD1fG6hJ3kL8zX0cV4bN7mQ2wE5rT",
    "https://updates.push.services.mozilla.com/wpush/v1/gAAAAABl9x2Kq7Lm3Np8Rs1Tu4Vw6Xy0Za5Bc9De2Fg7Hi1Jk4Lm8No3Pq6Rs0Tu5Vw9Xy2Za7Bc1De4Fg8Hi3Jk6Lm0No5Pq9Rs2Tu7Vw",
    "https://updates.push.services.mozilla.com/wpush/v2/gAAAAABm1a2Bb3Cc4Dd5Ee6Ff7Gg8Hh9Ii0Jj1Kk2Ll3Mm4Nn5Oo6Pp7Qq8Rr9Ss0Tt1Uu2Vv3Ww4Xx5Yy6Zz7_-aB3cD4eF5gH6iJ7kL8mN9o=",
];

/// A distinct scripted reply per endpoint.
const REPLIES: [DeliveryReply; 4] = [
    DeliveryReply {
        outcome: Outcome::Delivered,
        status: Some(201),
        retry_after_s: None,
    },
    DeliveryReply {
        outcome: Outcome::Gone,
        status: Some(410),
        retry_after_s: None,
    },
    DeliveryReply {
        outcome: Outcome::Retry,
        status: Some(429),
        retry_after_s: Some(120),
    },
    DeliveryReply {
        outcome: Outcome::Rejected,
        status: Some(403),
        retry_after_s: None,
    },
];

fn sa(ip: &str, port: u16) -> SocketAddr {
    SocketAddr::new(ip.parse::<IpAddr>().unwrap(), port)
}

fn lookup_answering(answer: Vec<SocketAddr>) -> (AddressLookup, Arc<Mutex<Vec<String>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    let lookup: AddressLookup =
        Arc::new(move |host: &str| -> Result<Vec<SocketAddr>, LookupError> {
            seen.lock().unwrap().push(host.to_string());
            Ok(answer.clone())
        });
    (lookup, asked)
}

fn request_for(endpoint: &str) -> DeliveryRequest {
    DeliveryRequest {
        endpoint: PushEndpoint::parse(endpoint).unwrap(),
        authorization: Zeroizing::new(
            "vapid t=eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.e30.c2ln, k=BAAA".to_string(),
        ),
        ttl_s: 43_200,
        urgency: Urgency::High,
        topic: Some("soosalerts".to_string()),
        body: Zeroizing::new(vec![0x5a; 120]),
    }
}

struct RecordingDeliverer {
    seen: Mutex<Vec<DeliveryRequest>>,
    reply: DeliveryReply,
}

impl Deliverer for RecordingDeliverer {
    fn deliver(&self, request: &DeliveryRequest) -> DeliveryReply {
        self.seen.lock().unwrap().push(request.clone());
        self.reply
    }
}

fn own_uid() -> u32 {
    let dir = tempfile::tempdir().unwrap();
    fs::metadata(dir.path()).unwrap().uid()
}

/// One frame through `serve_connection`; returns the decoded reply.
fn exchange(deliverer: Arc<RecordingDeliverer>, request: &DeliveryRequest) -> DeliveryReply {
    let (mut client, server) = UnixStream::pair().unwrap();
    let uid = own_uid();
    let handle = thread::spawn(move || serve_connection(server, uid, &*deliverer));
    client.write_all(&encode_request(request).unwrap()).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut prefix = [0u8; 4];
    client.read_exact(&mut prefix).unwrap();
    let mut payload = vec![0u8; u32::from_be_bytes(prefix) as usize];
    client.read_exact(&mut payload).unwrap();
    let mut rest = Vec::new();
    client.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty(), "one reply, then the connection closes");
    handle.join().unwrap();
    decode_reply(&payload).unwrap()
}

/// RAN9 (spec §11.2): each Android endpoint crosses the sender frame unchanged and gets its
/// scripted outcome; `filter_addresses` keeps a public answer (port 443) and refuses a private
/// one for its host, exactly as for Apple.
#[test]
fn test_ran_sender_accepts_android_endpoints() {
    for (endpoint, reply) in ANDROID_ENDPOINTS.iter().zip(REPLIES) {
        let fake = Arc::new(RecordingDeliverer {
            seen: Mutex::new(Vec::new()),
            reply,
        });
        let request = request_for(endpoint);
        assert_eq!(exchange(Arc::clone(&fake), &request), reply, "{endpoint}");
        let seen = fake.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "{endpoint}: the deliverer is reached once");
        assert_eq!(seen[0].endpoint.as_str(), *endpoint, "endpoint unchanged");
        assert!(seen[0] == request, "the decoded request is handed over");

        let host = request.endpoint.host();
        let (lookup, asked) = lookup_answering(vec![
            sa("142.250.74.106", 0),
            sa("2a00:1450:4007:80e::200a", 80),
        ]);
        assert_eq!(
            filter_addresses(host, &*lookup),
            Ok(vec![
                sa("142.250.74.106", 443),
                sa("2a00:1450:4007:80e::200a", 443)
            ]),
            "{host}: public answers kept on port 443"
        );
        assert_eq!(asked.lock().unwrap().as_slice(), [host.to_string()]);
        for private in [
            "10.0.0.7",
            "100.100.100.100",
            "127.0.0.1",
            "192.168.1.2",
            "fd7a:115c:a1e0::1",
        ] {
            let (lookup, _) = lookup_answering(vec![sa("142.250.74.106", 0), sa(private, 0)]);
            assert_eq!(
                filter_addresses(host, &*lookup),
                Err(Outcome::Refused),
                "{host}: a private answer {private} is refused"
            );
        }
    }
}
