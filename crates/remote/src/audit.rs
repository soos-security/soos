//! Fixed-text audit lines of the passkey ceremonies (`remote login accepted`, `passkey
//! registered`; ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey Authentication
//! for `soos-remote`", constraint C15).
//!
//! Each line is one `INFO` event with a constant message and no field. It is dispatched to
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
