//! Pure view slot and stream token of the live camera view (ADR 2026-10-07, architect spec
//! §7).
//!
//! One global slot: `Idle` (optionally in cooldown) → `Pending` (token reserved after a
//! verified passkey assertion, valid `CAMERA_VIEW_TOKEN_TTL_MS`) → `Starting` (stream request
//! accepted, first frame pending) → `Streaming` → `Idle` with a cooldown when pixels were
//! shown. Every method receives the current time; nothing here reads a clock, logs or does
//! I/O. The token is 32 CSPRNG bytes, single use, bound to its owner, compared in constant
//! time, zeroized on drop and never printed.

use std::fmt;
use std::time::Duration;

use base64ct::{Base64UrlUnpadded, Encoding};
use subtle::ConstantTimeEq;
use tokio::time::Instant;
use zeroize::{Zeroize, Zeroizing};

use crate::identity::PathClass;
use crate::{
    CAMERA_VIEW_COOLDOWN_MS, CAMERA_VIEW_TOKEN_B64_LEN, CAMERA_VIEW_TOKEN_BYTES,
    CAMERA_VIEW_TOKEN_TTL_MS,
};

/// 32-byte single-use stream token. Manual redacted `Debug`, zeroized on drop, compared in
/// constant time.
pub struct ViewToken([u8; CAMERA_VIEW_TOKEN_BYTES]);

impl ViewToken {
    /// Wraps CSPRNG bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; CAMERA_VIEW_TOKEN_BYTES]) -> Self {
        Self(bytes)
    }

    /// Unpadded base64url, exactly `CAMERA_VIEW_TOKEN_B64_LEN` characters.
    #[must_use]
    pub fn encode(&self) -> Zeroizing<String> {
        let mut buf = [0u8; CAMERA_VIEW_TOKEN_B64_LEN];
        let text = Zeroizing::new(
            Base64UrlUnpadded::encode(&self.0, &mut buf)
                .map(str::to_owned)
                .unwrap_or_default(),
        );
        buf.zeroize();
        text
    }

    /// Strict parse: exactly 43 characters of `[A-Za-z0-9_-]`, canonical (re-encoding equals
    /// the input), 32 bytes.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() != CAMERA_VIEW_TOKEN_B64_LEN
            || !text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return None;
        }
        let mut bytes = [0u8; CAMERA_VIEW_TOKEN_BYTES];
        let decoded_len = Base64UrlUnpadded::decode(text, &mut bytes)
            .ok()
            .map(<[u8]>::len);
        if decoded_len != Some(CAMERA_VIEW_TOKEN_BYTES) {
            bytes.zeroize();
            return None;
        }
        let token = Self(bytes);
        bytes.zeroize();
        let canonical = token.encode();
        if bool::from(canonical.as_bytes().ct_eq(text.as_bytes())) {
            Some(token)
        } else {
            None
        }
    }

    /// Constant-time equality.
    fn ct_equals(&self, other: &Self) -> bool {
        bool::from(self.0.ct_eq(&other.0))
    }
}

impl fmt::Debug for ViewToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ViewToken(<redacted>)")
    }
}

impl Drop for ViewToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Who may consume a token: the path class and, on the Funnel, the web-session hash.
#[derive(Clone, PartialEq, Eq)]
pub struct ViewOwner {
    /// Path class of the request that reserved the token.
    pub class: PathClass,
    /// Hash of the web session (Funnel); `None` on the tailnet.
    pub session: Option<[u8; 32]>,
}

impl ViewOwner {
    /// Class equal and session hashes equal in constant time.
    fn matches(&self, other: &Self) -> bool {
        if self.class != other.class {
            return false;
        }
        match (&self.session, &other.session) {
            (None, None) => true,
            (Some(a), Some(b)) => bool::from(a.ct_eq(b)),
            _ => false,
        }
    }
}

impl fmt::Debug for ViewOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ViewOwner(<redacted>)")
    }
}

/// Phase of the slot as reported by `GET /api/camera`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotPhase {
    /// No view and no cooldown.
    Idle,
    /// A token is reserved.
    Pending,
    /// The stream request was accepted, the first frame is pending.
    Starting,
    /// Frames are being sent.
    Streaming,
    /// A view ended recently.
    Cooldown,
}

/// Why the slot refused an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SlotError {
    /// `409 view_in_progress`.
    #[error("a camera view is in progress")]
    Busy,
    /// `429 camera_cooldown` (remaining ms in the body).
    #[error("camera view cooldown")]
    Cooldown {
        /// Remaining cooldown in milliseconds.
        remaining_ms: u64,
    },
    /// `403 view_token_rejected`.
    #[error("view token rejected")]
    TokenRejected,
}

/// Identifies one started view (never logged).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewTicket {
    /// View id.
    pub view: u64,
    /// End of the maximum view duration.
    pub ends_at: Instant,
}

/// Internal slot state.
enum SlotState {
    Idle {
        cooldown_until: Option<Instant>,
    },
    Pending {
        token: ViewToken,
        owner: ViewOwner,
        expires: Instant,
    },
    Starting {
        view: u64,
        ends_at: Instant,
        stop: bool,
    },
    Streaming {
        view: u64,
        ends_at: Instant,
        stop: bool,
    },
}

/// The single global view slot.
pub struct ViewSlot {
    state: SlotState,
    next_view: Option<u64>,
}

impl Default for ViewSlot {
    fn default() -> Self {
        Self::new()
    }
}

/// Milliseconds from `now` until `until`, rounded up (0 when elapsed).
fn remaining_ms(until: Instant, now: Instant) -> u64 {
    let left = until.saturating_duration_since(now);
    let whole = u64::try_from(left.as_millis()).unwrap_or(u64::MAX);
    if left.subsec_nanos().is_multiple_of(1_000_000) {
        whole
    } else {
        whole.saturating_add(1)
    }
}

impl ViewSlot {
    /// An idle slot without cooldown.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: SlotState::Idle {
                cooldown_until: None,
            },
            next_view: Some(1),
        }
    }

    /// Expires a `Pending` reservation whose `expires <= now` (Idle, no cooldown).
    fn expire(&mut self, now: Instant) {
        if let SlotState::Pending { expires, .. } = &self.state {
            if *expires <= now {
                self.state = SlotState::Idle {
                    cooldown_until: None,
                };
            }
        }
    }

    /// Pre-check without a change of state.
    ///
    /// # Errors
    ///
    /// [`SlotError::Cooldown`] in a cooldown, [`SlotError::Busy`] while a view is pending,
    /// starting or streaming.
    pub fn check(&mut self, now: Instant) -> Result<(), SlotError> {
        self.expire(now);
        match &self.state {
            SlotState::Idle { cooldown_until } => match cooldown_until {
                Some(until) if *until > now => Err(SlotError::Cooldown {
                    remaining_ms: remaining_ms(*until, now),
                }),
                _ => Ok(()),
            },
            SlotState::Pending { .. }
            | SlotState::Starting { .. }
            | SlotState::Streaming { .. } => Err(SlotError::Busy),
        }
    }

    /// Reserves the slot for `owner` with `token`, valid `CAMERA_VIEW_TOKEN_TTL_MS`.
    ///
    /// # Errors
    ///
    /// As [`Self::check`].
    pub fn reserve(
        &mut self,
        now: Instant,
        owner: ViewOwner,
        token: ViewToken,
    ) -> Result<(), SlotError> {
        self.check(now)?;
        self.state = SlotState::Pending {
            token,
            owner,
            expires: now
                .checked_add(Duration::from_millis(CAMERA_VIEW_TOKEN_TTL_MS))
                .unwrap_or(now),
        };
        Ok(())
    }

    /// Consumes the token: `Pending` with an equal token and owner, not expired → `Starting`
    /// with `ends_at = now + max_view`. A different token or owner keeps the reservation.
    ///
    /// # Errors
    ///
    /// [`SlotError::TokenRejected`] for a wrong, expired or absent token;
    /// [`SlotError::Busy`] when view ids are exhausted.
    pub fn begin(
        &mut self,
        now: Instant,
        owner: ViewOwner,
        presented: &ViewToken,
        max_view: Duration,
    ) -> Result<ViewTicket, SlotError> {
        self.expire(now);
        let SlotState::Pending {
            token,
            owner: reserved,
            ..
        } = &self.state
        else {
            return Err(SlotError::TokenRejected);
        };
        if !token.ct_equals(presented) || !reserved.matches(&owner) {
            return Err(SlotError::TokenRejected);
        }
        let Some(view) = self.next_view else {
            return Err(SlotError::Busy);
        };
        self.next_view = view.checked_add(1);
        let ends_at = now.checked_add(max_view).unwrap_or(now);
        // The reserved token is dropped (and zeroized) here: single use.
        self.state = SlotState::Starting {
            view,
            ends_at,
            stop: false,
        };
        Ok(ViewTicket { view, ends_at })
    }

    /// `Starting(view)` → `Streaming(view)`, keeping `ends_at` and the stop flag.
    pub fn mark_streaming(&mut self, view: u64) {
        if let SlotState::Starting {
            view: current,
            ends_at,
            stop,
        } = self.state
        {
            if current == view {
                self.state = SlotState::Streaming {
                    view,
                    ends_at,
                    stop,
                };
            }
        }
    }

    /// Stops what the slot holds: a `Pending` reservation is cancelled (Idle, no cooldown);
    /// a `Starting`/`Streaming` view gets a durable stop flag (kept until `end`). `false` when
    /// idle.
    pub fn stop(&mut self) -> bool {
        match &mut self.state {
            SlotState::Idle { .. } => false,
            SlotState::Pending { .. } => {
                self.state = SlotState::Idle {
                    cooldown_until: None,
                };
                true
            }
            SlotState::Starting { stop, .. } | SlotState::Streaming { stop, .. } => {
                *stop = true;
                true
            }
        }
    }

    /// Whether `view` must stop: its stop flag is set, or the slot no longer holds it.
    #[must_use]
    pub fn stop_requested(&self, view: u64) -> bool {
        match &self.state {
            SlotState::Starting {
                view: current,
                stop,
                ..
            }
            | SlotState::Streaming {
                view: current,
                stop,
                ..
            } => *current != view || *stop,
            SlotState::Idle { .. } | SlotState::Pending { .. } => true,
        }
    }

    /// Ends `view`: Idle, with a cooldown when it showed pixels. Another view id is a no-op.
    pub fn end(&mut self, now: Instant, view: u64, shown: bool) {
        let holds = matches!(
            &self.state,
            SlotState::Starting { view: current, .. } | SlotState::Streaming { view: current, .. }
                if *current == view
        );
        if holds {
            self.state = SlotState::Idle {
                cooldown_until: shown.then(|| {
                    now.checked_add(Duration::from_millis(CAMERA_VIEW_COOLDOWN_MS))
                        .unwrap_or(now)
                }),
            };
        }
    }

    /// Read-only phase and remaining cooldown (ms).
    #[must_use]
    pub fn phase(&self, now: Instant) -> (SlotPhase, Option<u64>) {
        match &self.state {
            SlotState::Idle { cooldown_until } => match cooldown_until {
                Some(until) if *until > now => {
                    (SlotPhase::Cooldown, Some(remaining_ms(*until, now)))
                }
                _ => (SlotPhase::Idle, None),
            },
            SlotState::Pending { expires, .. } => {
                if *expires <= now {
                    (SlotPhase::Idle, None)
                } else {
                    (SlotPhase::Pending, None)
                }
            }
            SlotState::Starting { .. } => (SlotPhase::Starting, None),
            SlotState::Streaming { .. } => (SlotPhase::Streaming, None),
        }
    }

    /// End of the maximum duration of `view`, if the slot holds it.
    #[must_use]
    pub fn ends_at(&self, view: u64) -> Option<Instant> {
        match &self.state {
            SlotState::Starting {
                view: current,
                ends_at,
                ..
            }
            | SlotState::Streaming {
                view: current,
                ends_at,
                ..
            } if *current == view => Some(*ends_at),
            _ => None,
        }
    }
}
