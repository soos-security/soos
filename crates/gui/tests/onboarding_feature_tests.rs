//! Contract (GitHub #379, PR 1 of #363, matrix OBT5–OBT7): the feature catalogue of the tour,
//! the state shown for each feature and the route of each on/off request.
//!
//! - Every feature of #363 has a step with plain, non-childish English wording.
//! - The state shown comes only from probed facts; an unknown fact is shown as unknown.
//! - Root-owned state changes only through a privileged `FeatureAction`; `soos-remote` keys can
//!   be turned off in the GUI and are turned back on with an exact, shown command (AS-1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::collections::HashSet;

use soos_gui::feature_control::{FeatureAction, RemoteSwitch, SystemFlag};
use soos_gui::onboarding::{
    toggle_route, Feature, FeatureSnapshot, FeatureState, RemoteConfigState, RemoteKeys,
    ToggleRoute, UnitState, FEATURES,
};

fn all_on() -> FeatureSnapshot {
    FeatureSnapshot {
        global_disabled: Some(false),
        presence_disabled: Some(false),
        network_lock_installed: Some(true),
        network_lock_rfkill_installed: Some(false),
        daemon_presence_enabled: Some(true),
        daemon_remote_view: Some(true),
        remote_unit: UnitState::Active,
        remote_config: RemoteConfigState::Keys(RemoteKeys {
            allow_unlock: true,
            allow_funnel: true,
            password_alerts: true,
            push_notifications: true,
            camera_view: true,
            camera_view_funnel: true,
            offline_alerts: true,
        }),
        running_as_root: false,
    }
}

const REMOTE_FEATURES: [(Feature, RemoteSwitch); 5] = [
    (Feature::RemoteUnlock, RemoteSwitch::Unlock),
    (Feature::InternetAccess, RemoteSwitch::Funnel),
    (Feature::PushAlerts, RemoteSwitch::Push),
    (Feature::CameraView, RemoteSwitch::CameraView),
    (Feature::OfflineAlerts, RemoteSwitch::OfflineAlerts),
];

// ---------------------------------------------------------------------------------------------
// Catalogue and wording
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_catalogue_lists_every_feature_of_the_epic_once() {
    let expected = [
        Feature::FaceUnlock,
        Feature::PresenceUnlock,
        Feature::PhoneCompanion,
        Feature::RemoteUnlock,
        Feature::InternetAccess,
        Feature::PushAlerts,
        Feature::CameraView,
        Feature::WifiLock,
        Feature::OfflineAlerts,
    ];
    assert_eq!(FEATURES, expected);
    let titles: HashSet<&str> = FEATURES.iter().map(|f| f.title()).collect();
    assert_eq!(titles.len(), FEATURES.len(), "titles must be unique");
}

#[test]
fn test_obt_wording_is_plain_english_and_not_childish() {
    let banned = [
        "awesome", "magic", "simply", "just ", "super", "amazing", "easy", "wow", "yay",
    ];
    for feature in FEATURES {
        let texts = [feature.title(), feature.explanation(), feature.when_off()];
        for text in texts {
            assert!(!text.is_empty(), "{feature:?} has an empty text");
            assert!(
                text.is_ascii(),
                "{feature:?}: non-ASCII text (emoji?): {text}"
            );
            assert!(!text.contains('!'), "{feature:?}: exclamation mark: {text}");
            let lower = text.to_ascii_lowercase();
            for word in banned {
                assert!(
                    !lower.contains(word),
                    "{feature:?}: banned word {word:?}: {text}"
                );
            }
        }
        assert!(
            feature.explanation().len() >= 80,
            "{feature:?}: the explanation must say what the feature does and when"
        );
        assert!(
            feature.title().len() <= 32,
            "{feature:?}: title too long for the step header"
        );
    }
}

#[test]
fn test_obt_presence_explanation_states_the_unlock_without_keypress() {
    let text = Feature::PresenceUnlock.explanation().to_ascii_lowercase();
    assert!(text.contains("unlock"), "{text}");
    assert!(text.contains("without"), "{text}");
}

// ---------------------------------------------------------------------------------------------
// State table
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_unknown_snapshot_shows_unknown_everywhere() {
    let snapshot = FeatureSnapshot::unknown();
    for feature in FEATURES {
        assert_eq!(
            snapshot.state(feature),
            FeatureState::Unknown,
            "{feature:?}"
        );
    }
}

#[test]
fn test_obt_all_on_snapshot_shows_on() {
    let snapshot = all_on();
    for feature in FEATURES {
        assert_eq!(snapshot.state(feature), FeatureState::On, "{feature:?}");
    }
}

#[test]
fn test_obt_global_flag_turns_face_and_presence_off() {
    let snapshot = FeatureSnapshot {
        global_disabled: Some(true),
        ..all_on()
    };
    assert_eq!(snapshot.state(Feature::FaceUnlock), FeatureState::Off);
    assert_eq!(snapshot.state(Feature::PresenceUnlock), FeatureState::Off);
}

#[test]
fn test_obt_presence_flag_turns_only_presence_off() {
    let snapshot = FeatureSnapshot {
        presence_disabled: Some(true),
        ..all_on()
    };
    assert_eq!(snapshot.state(Feature::FaceUnlock), FeatureState::On);
    assert_eq!(snapshot.state(Feature::PresenceUnlock), FeatureState::Off);
}

#[test]
fn test_obt_presence_unknown_when_a_flag_is_unknown_and_none_is_set() {
    for (global, presence) in [(None, Some(false)), (Some(false), None), (None, None)] {
        let snapshot = FeatureSnapshot {
            global_disabled: global,
            presence_disabled: presence,
            ..all_on()
        };
        assert_eq!(
            snapshot.state(Feature::PresenceUnlock),
            FeatureState::Unknown,
            "{global:?} {presence:?}"
        );
    }
    // A known set flag is decisive even when the other is unknown.
    let snapshot = FeatureSnapshot {
        global_disabled: None,
        presence_disabled: Some(true),
        ..all_on()
    };
    assert_eq!(snapshot.state(Feature::PresenceUnlock), FeatureState::Off);
}

#[test]
fn test_obt_companion_state_follows_the_user_unit() {
    for (unit, expected) in [
        (UnitState::Active, FeatureState::On),
        (UnitState::Inactive, FeatureState::Off),
        (UnitState::NotInstalled, FeatureState::NotInstalled),
        (UnitState::Masked, FeatureState::Off),
        (UnitState::Unknown, FeatureState::Unknown),
    ] {
        let snapshot = FeatureSnapshot {
            remote_unit: unit,
            ..all_on()
        };
        assert_eq!(
            snapshot.state(Feature::PhoneCompanion),
            expected,
            "{unit:?}"
        );
    }
}

#[test]
fn test_obt_remote_key_states_follow_each_key() {
    for (feature, switch) in REMOTE_FEATURES {
        let mut keys = RemoteKeys {
            allow_unlock: true,
            allow_funnel: true,
            password_alerts: true,
            push_notifications: true,
            camera_view: true,
            camera_view_funnel: true,
            offline_alerts: true,
        };
        match switch {
            RemoteSwitch::Unlock => keys.allow_unlock = false,
            RemoteSwitch::Funnel => keys.allow_funnel = false,
            RemoteSwitch::Push => keys.push_notifications = false,
            RemoteSwitch::CameraView => keys.camera_view = false,
            RemoteSwitch::OfflineAlerts => keys.offline_alerts = false,
        }
        let snapshot = FeatureSnapshot {
            remote_config: RemoteConfigState::Keys(keys),
            ..all_on()
        };
        assert_eq!(snapshot.state(feature), FeatureState::Off, "{feature:?}");
        for (other, _) in REMOTE_FEATURES {
            if other != feature {
                assert_eq!(snapshot.state(other), FeatureState::On, "{other:?}");
            }
        }
    }
}

#[test]
fn test_obt_remote_keys_not_installed_or_unknown() {
    for (feature, _) in REMOTE_FEATURES {
        let not_installed_unit = FeatureSnapshot {
            remote_unit: UnitState::NotInstalled,
            ..all_on()
        };
        assert_eq!(
            not_installed_unit.state(feature),
            FeatureState::NotInstalled
        );
        let missing_config = FeatureSnapshot {
            remote_config: RemoteConfigState::Missing,
            ..all_on()
        };
        assert_eq!(missing_config.state(feature), FeatureState::NotInstalled);
        for config in [RemoteConfigState::Invalid, RemoteConfigState::Unknown] {
            let snapshot = FeatureSnapshot {
                remote_config: config,
                ..all_on()
            };
            assert_eq!(snapshot.state(feature), FeatureState::Unknown, "{config:?}");
        }
    }
}

#[test]
fn test_obt_wifi_lock_state_follows_the_rule_file() {
    for (installed, expected) in [
        (Some(true), FeatureState::On),
        (Some(false), FeatureState::Off),
        (None, FeatureState::Unknown),
    ] {
        let snapshot = FeatureSnapshot {
            network_lock_installed: installed,
            ..all_on()
        };
        assert_eq!(snapshot.state(Feature::WifiLock), expected);
    }
}

// ---------------------------------------------------------------------------------------------
// Toggle routing
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_root_flags_route_through_privileged_flag_actions() {
    let s = all_on();
    assert_eq!(
        toggle_route(&s, Feature::FaceUnlock, false),
        ToggleRoute::Action(FeatureAction::SetSystemFlag {
            flag: SystemFlag::Global,
            present: true
        })
    );
    assert_eq!(
        toggle_route(&s, Feature::FaceUnlock, true),
        ToggleRoute::Action(FeatureAction::SetSystemFlag {
            flag: SystemFlag::Global,
            present: false
        })
    );
    assert_eq!(
        toggle_route(&s, Feature::PresenceUnlock, false),
        ToggleRoute::Action(FeatureAction::SetSystemFlag {
            flag: SystemFlag::Presence,
            present: true
        })
    );
    assert_eq!(
        toggle_route(&s, Feature::PresenceUnlock, true),
        ToggleRoute::Action(FeatureAction::SetSystemFlag {
            flag: SystemFlag::Presence,
            present: false
        })
    );
    // Root-owned state never depends on the GUI's own uid: the same route as root.
    let root = FeatureSnapshot {
        running_as_root: true,
        ..all_on()
    };
    assert_eq!(
        toggle_route(&root, Feature::PresenceUnlock, false),
        toggle_route(&s, Feature::PresenceUnlock, false)
    );
}

#[test]
fn test_obt_wifi_lock_routes_through_the_admin_cli_action() {
    let s = all_on();
    assert_eq!(
        toggle_route(&s, Feature::WifiLock, false),
        ToggleRoute::Action(FeatureAction::SetNetworkLock { installed: false })
    );
    assert_eq!(
        toggle_route(&s, Feature::WifiLock, true),
        ToggleRoute::Action(FeatureAction::SetNetworkLock { installed: true })
    );
}

#[test]
fn test_obt_wifi_lock_on_never_drops_an_installed_or_unknown_rfkill_rule() {
    // `soos-admin network-lock install` without `--rfkill` removes the rfkill rule: the GUI
    // only runs it when the rfkill rule is known to be absent.
    for rfkill in [Some(true), None] {
        let s = FeatureSnapshot {
            network_lock_installed: Some(false),
            network_lock_rfkill_installed: rfkill,
            ..all_on()
        };
        match toggle_route(&s, Feature::WifiLock, true) {
            ToggleRoute::Command(text) => {
                assert!(
                    text.contains("soos-admin network-lock install --rfkill"),
                    "{rfkill:?}: {text}"
                );
            }
            other => panic!("{rfkill:?}: expected a command, got {other:?}"),
        }
        // Off is always the admin CLI action (it removes both rules).
        assert_eq!(
            toggle_route(&s, Feature::WifiLock, false),
            ToggleRoute::Action(FeatureAction::SetNetworkLock { installed: false })
        );
    }
}

#[test]
fn test_obt_presence_on_is_unavailable_while_face_unlock_is_off() {
    let s = FeatureSnapshot {
        global_disabled: Some(true),
        presence_disabled: Some(true),
        ..all_on()
    };
    assert!(matches!(
        toggle_route(&s, Feature::PresenceUnlock, true),
        ToggleRoute::Unavailable(_)
    ));
    let when_off = Feature::FaceUnlock.when_off().to_ascii_lowercase();
    assert!(when_off.contains("presence"), "{when_off}");
}

#[test]
fn test_obt_daemon_presence_key_off_shows_off_and_the_daemon_command() {
    let s = FeatureSnapshot {
        daemon_presence_enabled: Some(false),
        ..all_on()
    };
    assert_eq!(s.state(Feature::PresenceUnlock), FeatureState::Off);
    assert_eq!(s.state(Feature::FaceUnlock), FeatureState::On);
    match toggle_route(&s, Feature::PresenceUnlock, true) {
        ToggleRoute::Command(text) => {
            assert!(text.contains("[presence]"), "{text}");
            assert!(text.contains("/etc/soos/daemon.toml"), "{text}");
            assert!(
                text.contains("systemctl restart soos-daemon.service"),
                "{text}"
            );
        }
        other => panic!("expected a command, got {other:?}"),
    }
    let unknown = FeatureSnapshot {
        daemon_presence_enabled: None,
        ..all_on()
    };
    assert_eq!(
        unknown.state(Feature::PresenceUnlock),
        FeatureState::Unknown
    );
}

#[test]
fn test_obt_camera_view_needs_the_daemon_remote_view() {
    let blocked = FeatureSnapshot {
        daemon_remote_view: Some(false),
        ..all_on()
    };
    assert_eq!(blocked.state(Feature::CameraView), FeatureState::Off);
    // The phone-side key can still be turned off in the GUI.
    assert_eq!(
        toggle_route(&blocked, Feature::CameraView, false),
        ToggleRoute::Action(FeatureAction::DisableRemote(RemoteSwitch::CameraView))
    );
    match toggle_route(&blocked, Feature::CameraView, true) {
        ToggleRoute::Command(text) => {
            assert!(text.contains("remote_view"), "{text}");
            assert!(text.contains("/etc/soos/daemon.toml"), "{text}");
        }
        other => panic!("expected a command, got {other:?}"),
    }
    let unknown = FeatureSnapshot {
        daemon_remote_view: None,
        ..all_on()
    };
    assert_eq!(unknown.state(Feature::CameraView), FeatureState::Unknown);
}

#[test]
fn test_obt_turn_on_commands_name_their_prerequisites() {
    let s = all_on();
    let expect = [
        (Feature::RemoteUnlock, vec!["rp_id", "passkey"]),
        (Feature::InternetAccess, vec!["rp_id", "Funnel"]),
        (
            Feature::PushAlerts,
            vec!["password_alerts = true", "rp_id", "soos-push-sender"],
        ),
        (
            Feature::CameraView,
            vec!["rp_id", "remote_view", "/etc/soos/daemon.toml"],
        ),
    ];
    for (feature, needles) in expect {
        let ToggleRoute::Command(text) = toggle_route(&s, feature, true) else {
            panic!("{feature:?}: expected a command");
        };
        for needle in needles {
            assert!(
                text.contains(needle),
                "{feature:?}: missing {needle:?} in {text}"
            );
        }
    }
}

#[test]
fn test_obt_masked_companion_routes() {
    let s = FeatureSnapshot {
        remote_unit: UnitState::Masked,
        ..all_on()
    };
    match toggle_route(&s, Feature::PhoneCompanion, true) {
        ToggleRoute::Command(text) => {
            assert!(
                text.contains("systemctl --user unmask soos-remote.service"),
                "{text}"
            );
        }
        other => panic!("expected a command, got {other:?}"),
    }
    assert!(matches!(
        toggle_route(&s, Feature::PhoneCompanion, false),
        ToggleRoute::Unavailable(_)
    ));
}

#[test]
fn test_obt_companion_routes_through_the_user_manager() {
    let s = all_on();
    assert_eq!(
        toggle_route(&s, Feature::PhoneCompanion, false),
        ToggleRoute::Action(FeatureAction::SetCompanion { enabled: false })
    );
    assert_eq!(
        toggle_route(&s, Feature::PhoneCompanion, true),
        ToggleRoute::Action(FeatureAction::SetCompanion { enabled: true })
    );
}

#[test]
fn test_obt_companion_as_root_shows_the_user_command() {
    let root = FeatureSnapshot {
        running_as_root: true,
        ..all_on()
    };
    match toggle_route(&root, Feature::PhoneCompanion, false) {
        ToggleRoute::Command(text) => {
            assert!(
                text.contains("systemctl --user disable --now soos-remote.service"),
                "{text}"
            );
        }
        other => panic!("expected a command, got {other:?}"),
    }
    match toggle_route(&root, Feature::PhoneCompanion, true) {
        ToggleRoute::Command(text) => {
            assert!(
                text.contains("systemctl --user enable --now soos-remote.service"),
                "{text}"
            );
        }
        other => panic!("expected a command, got {other:?}"),
    }
}

#[test]
fn test_obt_companion_not_installed_is_unavailable() {
    let s = FeatureSnapshot {
        remote_unit: UnitState::NotInstalled,
        ..all_on()
    };
    for on in [false, true] {
        assert!(matches!(
            toggle_route(&s, Feature::PhoneCompanion, on),
            ToggleRoute::Unavailable(_)
        ));
    }
}

#[test]
fn test_obt_remote_keys_turn_off_in_the_gui_and_on_by_command() {
    let s = all_on();
    for (feature, switch) in REMOTE_FEATURES {
        assert_eq!(
            toggle_route(&s, feature, false),
            ToggleRoute::Action(FeatureAction::DisableRemote(switch)),
            "{feature:?}"
        );
        match toggle_route(&s, feature, true) {
            ToggleRoute::Command(text) => {
                assert!(
                    text.contains(&format!("{} = true", switch.key())),
                    "{feature:?}: {text}"
                );
                assert!(text.contains("~/.config/soos/remote.toml"), "{text}");
                assert!(
                    text.contains("systemctl --user restart soos-remote.service"),
                    "{text}"
                );
            }
            other => panic!("{feature:?}: expected a command, got {other:?}"),
        }
    }
}

#[test]
fn test_obt_remote_keys_as_root_or_without_config_show_the_off_command() {
    let root = FeatureSnapshot {
        running_as_root: true,
        ..all_on()
    };
    for config in [
        RemoteConfigState::Unknown,
        RemoteConfigState::Invalid,
        RemoteConfigState::Missing,
    ] {
        let no_keys = FeatureSnapshot {
            remote_config: config,
            ..all_on()
        };
        for snapshot in [&root, &no_keys] {
            for (feature, switch) in REMOTE_FEATURES {
                match toggle_route(snapshot, feature, false) {
                    ToggleRoute::Command(text) => {
                        assert!(
                            text.contains(&format!("{} = false", switch.key())),
                            "{feature:?}: {text}"
                        );
                    }
                    other => panic!("{feature:?}: expected a command, got {other:?}"),
                }
            }
        }
    }
}

#[test]
fn test_obt_offline_alerts_have_an_off_route_and_need_push_to_turn_on() {
    // Offline alerts shipped with GitHub #376 (`offline_alerts`, ADR 2026-10-13 "Offline History
    // and Back-Online Push Notifications"): every feature keeps an off route.
    let s = all_on();
    assert_eq!(
        toggle_route(&s, Feature::OfflineAlerts, false),
        ToggleRoute::Action(FeatureAction::DisableRemote(RemoteSwitch::OfflineAlerts))
    );
    match toggle_route(&s, Feature::OfflineAlerts, true) {
        ToggleRoute::Command(text) => {
            assert!(text.contains("offline_alerts = true"), "{text}");
            // soos-remote refuses `offline_alerts` without push notifications.
            assert!(text.contains("push_notifications = true"), "{text}");
        }
        other => panic!("expected a command, got {other:?}"),
    }
    assert_ne!(
        Feature::OfflineAlerts.when_off(),
        "Not available in this version."
    );
}

#[test]
fn test_obt_shown_commands_carry_no_user_data() {
    // Commands are fixed strings: no login, host name, uid or path of this machine.
    let s = FeatureSnapshot {
        running_as_root: true,
        ..all_on()
    };
    for feature in FEATURES {
        for on in [false, true] {
            if let ToggleRoute::Command(text) = toggle_route(&s, feature, on) {
                assert!(!text.contains("/home/"), "{text}");
                assert!(!text.contains("ts.net"), "{text}");
                assert!(text.is_ascii(), "{text}");
            }
        }
    }
}

#[test]
fn test_obt_presence_off_on_both_sides_shows_the_daemon_command_first() {
    let s = FeatureSnapshot {
        presence_disabled: Some(true),
        daemon_presence_enabled: Some(false),
        ..all_on()
    };
    assert_eq!(s.state(Feature::PresenceUnlock), FeatureState::Off);
    match toggle_route(&s, Feature::PresenceUnlock, true) {
        ToggleRoute::Command(text) => {
            let daemon = text
                .find("/etc/soos/daemon.toml")
                .expect("daemon key named");
            let flag = text.find("/etc/soos/presence.disable").expect("flag named");
            assert!(daemon < flag, "{text}");
        }
        other => panic!("expected a command, got {other:?}"),
    }
}
