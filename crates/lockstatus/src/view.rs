//! Lock-screen view of one poll and its presentation policy.

use soos_protocol::{LockStateResponse, LockWidgetState};

/// What one poll learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewState {
    /// The daemon could not be asked or refused to answer (transport error, refusal, old daemon).
    Unavailable,
    /// No scan in progress and nothing to report.
    Idle,
    /// A face scan of the user is in progress.
    Scanning,
    /// The last scan ended without verifying the face, whatever the reason.
    NotRecognized,
}

/// One poll result: the state and the daemon's per-UID transition counter (`0` when unavailable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LockView {
    /// Polled state.
    pub state: ViewState,
    /// Daemon transition counter; a lower value starts a new epoch.
    pub sequence: u64,
}

/// The only states a locked screen may show (owner decision Q2 of 2026-10-13 on #352).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Presented {
    /// Nothing to show (also every transport error).
    Idle,
    /// A scan is running.
    Scanning,
    /// The last scan failed (every reason collapsed).
    NotRecognized,
}

impl LockView {
    /// The view of every failed poll.
    pub const UNAVAILABLE: Self = Self {
        state: ViewState::Unavailable,
        sequence: 0,
    };

    /// Maps a decoded daemon answer; a response of another protocol version is unavailable.
    #[must_use]
    pub fn from_response(response: &LockStateResponse) -> Self {
        if !response.is_current() {
            return Self::UNAVAILABLE;
        }
        let state = match response.state {
            LockWidgetState::Idle => ViewState::Idle,
            LockWidgetState::Scanning => ViewState::Scanning,
            LockWidgetState::NotRecognized => ViewState::NotRecognized,
        };
        Self {
            state,
            sequence: response.sequence,
        }
    }

    /// The state shown at the locked screen: unavailable renders as idle.
    #[must_use]
    pub fn presented(&self) -> Presented {
        match self.state {
            ViewState::Unavailable | ViewState::Idle => Presented::Idle,
            ViewState::Scanning => Presented::Scanning,
            ViewState::NotRecognized => Presented::NotRecognized,
        }
    }

    /// Whether the daemon answered (only `false` for [`ViewState::Unavailable`]).
    #[must_use]
    pub fn available(&self) -> bool {
        self.state != ViewState::Unavailable
    }
}
