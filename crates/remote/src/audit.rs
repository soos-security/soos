//! Fixed-text audit lines of the passkey ceremonies and of the remote unlock (`remote login
//! accepted`, `passkey registered`, `remote unlock requested`; ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey Authentication
//! for `soos-remote`", constraint C15), and of the failed-password alerts (`password alerts
//! active`, `password alerts unavailable`, `password alert acknowledgement not persisted`;
//! ADR 2026-10-06 "Failed-Password Alerts in `soos-remote` From the System Journal", A-11), and of
//! Web Push (`push subscription added`, `push subscription removed`, `push delivery failed`,
//! `push sender unavailable`, `push notifications unavailable`; ADR 2026-10-06 "Web Push
//! Notifications for Failed-Password Alerts Through a Separate Sender Unit", W-14).
//!
//! Each line is one `INFO` or `WARN` event with a constant message and no field. It is dispatched to
//! the current subscriber after its own `enabled` check instead of through a `tracing` macro:
//! a macro callsite caches its interest process-wide, so a callsite first reached on a thread
//! without a subscriber would stay disabled for a subscriber scoped to another thread (the
//! audit line would be lost). Nothing here ever carries a value.

use tracing::callsite::{Callsite, Identifier};
use tracing::field::{FieldSet, Value};
use tracing::metadata::Kind;
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata};

/// Static callsite of one audit line.
struct AuditCallsite(&'static Metadata<'static>);

impl Callsite for AuditCallsite {
    fn set_interest(&self, _interest: Interest) {}

    fn metadata(&self) -> &Metadata<'_> {
        self.0
    }
}

/// The only field of an audit event.
const FIELDS: &[&str] = &["message"];

static LOGIN_CALLSITE: AuditCallsite = AuditCallsite(&LOGIN_META);
static LOGIN_META: Metadata<'static> = Metadata::new(
    "remote login accepted",
    "soos_remote::audit",
    Level::INFO,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&LOGIN_CALLSITE)),
    Kind::EVENT,
);

static REGISTERED_CALLSITE: AuditCallsite = AuditCallsite(&REGISTERED_META);
static REGISTERED_META: Metadata<'static> = Metadata::new(
    "passkey registered",
    "soos_remote::audit",
    Level::INFO,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&REGISTERED_CALLSITE)),
    Kind::EVENT,
);

static UNLOCK_CALLSITE: AuditCallsite = AuditCallsite(&UNLOCK_META);
static UNLOCK_META: Metadata<'static> = Metadata::new(
    "remote unlock requested",
    "soos_remote::audit",
    Level::INFO,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&UNLOCK_CALLSITE)),
    Kind::EVENT,
);

static ALERTS_ACTIVE_CALLSITE: AuditCallsite = AuditCallsite(&ALERTS_ACTIVE_META);
static ALERTS_ACTIVE_META: Metadata<'static> = Metadata::new(
    "password alerts active",
    "soos_remote::audit",
    Level::INFO,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&ALERTS_ACTIVE_CALLSITE)),
    Kind::EVENT,
);

static ALERTS_UNAVAILABLE_CALLSITE: AuditCallsite = AuditCallsite(&ALERTS_UNAVAILABLE_META);
static ALERTS_UNAVAILABLE_META: Metadata<'static> = Metadata::new(
    "password alerts unavailable",
    "soos_remote::audit",
    Level::WARN,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&ALERTS_UNAVAILABLE_CALLSITE)),
    Kind::EVENT,
);

static ALERTS_ACK_CALLSITE: AuditCallsite = AuditCallsite(&ALERTS_ACK_META);
static ALERTS_ACK_META: Metadata<'static> = Metadata::new(
    "password alert acknowledgement not persisted",
    "soos_remote::audit",
    Level::WARN,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&ALERTS_ACK_CALLSITE)),
    Kind::EVENT,
);

static PUSH_ADDED_CALLSITE: AuditCallsite = AuditCallsite(&PUSH_ADDED_META);
static PUSH_ADDED_META: Metadata<'static> = Metadata::new(
    "push subscription added",
    "soos_remote::audit",
    Level::INFO,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&PUSH_ADDED_CALLSITE)),
    Kind::EVENT,
);

static PUSH_REMOVED_CALLSITE: AuditCallsite = AuditCallsite(&PUSH_REMOVED_META);
static PUSH_REMOVED_META: Metadata<'static> = Metadata::new(
    "push subscription removed",
    "soos_remote::audit",
    Level::INFO,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&PUSH_REMOVED_CALLSITE)),
    Kind::EVENT,
);

static PUSH_FAILED_CALLSITE: AuditCallsite = AuditCallsite(&PUSH_FAILED_META);
static PUSH_FAILED_META: Metadata<'static> = Metadata::new(
    "push delivery failed",
    "soos_remote::audit",
    Level::WARN,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&PUSH_FAILED_CALLSITE)),
    Kind::EVENT,
);

static PUSH_SENDER_CALLSITE: AuditCallsite = AuditCallsite(&PUSH_SENDER_META);
static PUSH_SENDER_META: Metadata<'static> = Metadata::new(
    "push sender unavailable",
    "soos_remote::audit",
    Level::WARN,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&PUSH_SENDER_CALLSITE)),
    Kind::EVENT,
);

static PUSH_UNAVAILABLE_CALLSITE: AuditCallsite = AuditCallsite(&PUSH_UNAVAILABLE_META);
static PUSH_UNAVAILABLE_META: Metadata<'static> = Metadata::new(
    "push notifications unavailable",
    "soos_remote::audit",
    Level::WARN,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    FieldSet::new(FIELDS, Identifier(&PUSH_UNAVAILABLE_CALLSITE)),
    Kind::EVENT,
);

/// Dispatches one audit event whose message is the metadata name.
fn emit(meta: &'static Metadata<'static>) {
    tracing::dispatcher::get_default(|dispatch| {
        if !dispatch.enabled(meta) {
            return;
        }
        let fields = meta.fields();
        let Some(field) = fields.field("message") else {
            return;
        };
        let message: &str = meta.name();
        let values = [(&field, Some(&message as &dyn Value))];
        dispatch.event(&Event::new(meta, &fields.value_set(&values)));
    });
}

/// `INFO remote login accepted` (one line per accepted Funnel login).
pub fn login_accepted() {
    emit(&LOGIN_META);
}

/// `INFO passkey registered` (one line per stored passkey).
pub fn passkey_registered() {
    emit(&REGISTERED_META);
}

/// `INFO remote unlock requested` (one line per accepted unlock, right before
/// `UnlockSession`).
pub fn unlock_requested() {
    emit(&UNLOCK_META);
}

/// `INFO password alerts active` (once per transition to `active`).
pub fn alerts_active() {
    emit(&ALERTS_ACTIVE_META);
}

/// `WARN password alerts unavailable` (once per transition to `unavailable`).
pub fn alerts_unavailable() {
    emit(&ALERTS_UNAVAILABLE_META);
}

/// `WARN password alert acknowledgement not persisted` (an invalid acknowledgement file at
/// start, or a failed write, once per failure transition).
pub fn alerts_ack_not_persisted() {
    emit(&ALERTS_ACK_META);
}

/// `INFO push subscription added` (one line per stored new push subscription).
pub fn push_subscription_added() {
    emit(&PUSH_ADDED_META);
}

/// `INFO push subscription removed` (one line per removed push subscription).
pub fn push_subscription_removed() {
    emit(&PUSH_REMOVED_META);
}

/// `WARN push delivery failed` (once per transition into delivery failure).
pub fn push_delivery_failed() {
    emit(&PUSH_FAILED_META);
}

/// `WARN push sender unavailable` (once per transition to an unreachable sender).
pub fn push_sender_unavailable() {
    emit(&PUSH_SENDER_META);
}

/// `WARN push notifications unavailable` (once, when the push store cannot be used at start).
pub fn push_unavailable() {
    emit(&PUSH_UNAVAILABLE_META);
}
