/*
 * soos lock-screen extension (GitHub 405, PR 4 of the lock-screen epic; ADR 2026-10-17
 * "GNOME Shell Lock-Screen Extension"; user documentation Docs/LOCKSCREEN_WIDGET.md).
 *
 * While the session is locked, runs `soos-lockstatus --watch` (fixed argv, no shell), reads its
 * JSON lines with a bounded length and shows one non-reactive card on the lock screen: a ring,
 * the pink star and the words "Scanning" or "Not recognized". Only three states exist (Idle,
 * Scanning, Not recognized); `available: false` looks exactly like Idle. The extension reads a
 * coarse state and nothing else: it cannot unlock, start a scan, wake the camera or send input,
 * and it connects no input signal at all.
 *
 * Kill switch: when /etc/soos/lockwidget.disable exists at lock time, nothing is shown and no
 * process is started. Reduced motion follows org.gnome.desktop.interface enable-animations
 * through the shell's St.Settings mirror.
 */

import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import St from 'gi://St';
import Clutter from 'gi://Clutter';

import * as Layout from 'resource:///org/gnome/shell/ui/layout.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

/* Root-owned locations only: no PATH lookup, no home directory. */
const SOOS_LOCKSTATUS_CANDIDATES = ['/usr/bin/soos-lockstatus', '/usr/local/bin/soos-lockstatus'];
const KILL_SWITCH_PATH = '/etc/soos/lockwidget.disable';

/* The `--watch` line contract (Docs/LOCKSCREEN_WIDGET.md section 6), keys sorted. */
const WATCH_KEYS = ['available', 'sequence', 'state'];
const STATES = ['idle', 'scanning', 'not_recognized'];
/* A bridge line is at most 96 bytes; anything longer is a protocol error. */
const MAX_LINE_BYTES = 256;
const READ_CHUNK_BYTES = 256;
const RESPAWN_DELAYS_MS = [1000, 2000, 4000, 8000];
/* A scan older than this reads Idle in the daemon (Docs/IPC_PROTOCOL.md section 14). */
const SCAN_MAX_AGE_MS = 10000;

/* Motion tokens mirrored from Docs/UI_MOTION.md (as in soos-lock-widget). */
const T_SHORT_MS = 180;
const T_MEDIUM_MS = 260;
const T_PENDING_MS = 1200;
const SLIDE_PX = 8;
const PENDING_LOOP_MAX_MS = 4800;

/* The two spikes of the brand star (STAR_SPIKES of crates/gui/src/brand.rs). */
const STAR_VIEWBOX = 508;

const LABELS = {idle: '', scanning: 'Scanning', not_recognized: 'Not recognized'};
const RING_CLASSES = {
    idle: 'soos-idle',
    scanning: 'soos-scanning',
    not_recognized: 'soos-not-recognized',
};
const IDLE_VIEW = {state: 'idle', available: false, sequence: 0};

/**
 * Parses one `--watch` line; returns null for anything but an object with exactly the three
 * keys and valid values. Only `state`, `available` and `sequence` are ever read.
 *
 * @param {string} text - one line without its newline
 * @returns {object|null} the view
 */
function parseLine(text) {
    let value;
    try {
        value = JSON.parse(text);
    } catch {
        return null;
    }
    if (value === null || typeof value !== 'object' || Array.isArray(value))
        return null;
    const keys = Object.keys(value).sort();
    if (keys.length !== WATCH_KEYS.length)
        return null;
    for (let i = 0; i < WATCH_KEYS.length; i++) {
        if (keys[i] !== WATCH_KEYS[i])
            return null;
    }
    if (!STATES.includes(value.state) || typeof value.available !== 'boolean')
        return null;
    if (!Number.isSafeInteger(value.sequence) || value.sequence < 0)
        return null;
    return {state: value.state, available: value.available, sequence: value.sequence};
}

/**
 * The state drawn on the screen: a transport error is drawn exactly like Idle.
 *
 * @param {object} view - a parsed view
 * @returns {string} one of STATES
 */
function present(view) {
    return view.available ? view.state : 'idle';
}

/**
 * @returns {number} CLOCK_MONOTONIC in milliseconds
 */
function nowMs() {
    return GLib.get_monotonic_time() / 1000;
}

/**
 * The first candidate that is a root-owned regular file, not writable by group or others, and
 * executable; null when none is.
 *
 * @returns {string|null} absolute path of soos-lockstatus
 */
function findLockstatus() {
    for (const path of SOOS_LOCKSTATUS_CANDIDATES) {
        try {
            const info = Gio.File.new_for_path(path).query_info(
                'standard::type,unix::uid,unix::mode,access::can-execute',
                Gio.FileQueryInfoFlags.NONE, null);
            if (info.get_file_type() === Gio.FileType.REGULAR &&
                info.get_attribute_uint32('unix::uid') === 0 &&
                (info.get_attribute_uint32('unix::mode') & 0o022) === 0 &&
                info.get_attribute_boolean('access::can-execute'))
                return path;
        } catch {
            /* Missing or unreadable: try the next candidate. */
        }
    }
    return null;
}

/**
 * Draws the brand star in the area's foreground colour (CSS `color`).
 *
 * @param {St.DrawingArea} area - the star actor
 */
function drawStar(area) {
    const [width, height] = area.get_surface_size();
    const color = area.get_theme_node().get_foreground_color();
    const cr = area.get_context();
    try {
        const size = Math.min(width, height);
        cr.translate((width - size) / 2, (height - size) / 2);
        cr.scale(size / STAR_VIEWBOX, size / STAR_VIEWBOX);
        cr.setSourceRGBA(color.red / 255, color.green / 255, color.blue / 255, color.alpha / 255);
        cr.moveTo(254, 0);
        cr.curveTo(254, 140.28, 367.72, 254, 508, 254);
        cr.lineTo(254, 254);
        cr.closePath();
        cr.moveTo(0, 254);
        cr.curveTo(140.28, 254, 254, 367.72, 254, 508);
        cr.lineTo(254, 254);
        cr.lineTo(0, 254);
        cr.closePath();
        cr.fill();
    } finally {
        cr.$dispose();
    }
}

export default class SoosLockscreenExtension extends Extension {
    enable() {
        try {
            this._view = IDLE_VIEW;
            this._frame = null;
            this._card = null;
            this._ring = null;
            this._star = null;
            this._label = null;
            this._bridge = null;
            this._respawnId = 0;
            this._respawnIndex = 0;
            this._turnId = 0;
            this._starAngle = 0;
            this._scanStartMs = 0;
            this._scanSequence = -1;
            this._shakenSequence = -1;
            this._sessionId = 0;
            this._animationsId = 0;
            this._settings = St.Settings.get();
            this._sessionId = Main.sessionMode.connect('updated', () => {
                try {
                    this._onSessionUpdated();
                } catch {
                    console.warn('soos-lockscreen: session update failed');
                }
            });
            this._animationsId = this._settings.connect('notify::enable-animations', () => {
                try {
                    this._onAnimationsChanged();
                } catch {
                    console.warn('soos-lockscreen: animation setting update failed');
                }
            });
            this._sync();
        } catch {
            console.warn('soos-lockscreen: enable failed');
            this._stopAll();
        }
    }

    disable() {
        // This extension declares the unlock-dialog session mode because its only purpose is to
        // show read-only face-scan status on the lock screen and the unlock dialog, so it must
        // stay enabled while the session is locked. It cannot unlock and connects no input
        // signal. Everything it created is removed here: the bridge process and its cancellable,
        // every GLib source, both signal handlers and the card actor.
        try {
            if (this._sessionId)
                Main.sessionMode.disconnect(this._sessionId);
        } catch {
            console.warn('soos-lockscreen: disconnecting the session signal failed');
        }
        this._sessionId = 0;
        try {
            if (this._animationsId && this._settings)
                this._settings.disconnect(this._animationsId);
        } catch {
            console.warn('soos-lockscreen: disconnecting the animation signal failed');
        }
        this._animationsId = 0;
        this._removeSources();
        this._stopBridge();
        this._destroyCard();
        this._settings = null;
        this._view = IDLE_VIEW;
    }

    _onSessionUpdated() {
        try {
            this._sync();
        } catch {
            console.warn('soos-lockscreen: session update failed');
            this._stopAll();
        }
    }

    _onAnimationsChanged() {
        try {
            this._applyMotionClass();
        } catch {
            console.warn('soos-lockscreen: animation setting update failed');
        }
    }

    /* Shows the card while locked (and the kill switch is absent), removes it otherwise. */
    _sync() {
        try {
            const wanted = Main.sessionMode.isLocked && !this._killSwitchSet();
            if (wanted && this._card === null) {
                this._buildCard();
                this._startBridge();
            } else if (!wanted && this._card !== null) {
                this._stopAll();
            }
        } catch {
            console.warn('soos-lockscreen: showing the card failed');
            this._stopAll();
        }
    }

    _killSwitchSet() {
        return GLib.file_test(KILL_SWITCH_PATH, GLib.FileTest.EXISTS);
    }

    _reducedMotion() {
        return !this._settings || !this._settings.enable_animations;
    }

    _buildCard() {
        /* Covers the primary monitor, like the shell's own unlock dialog box. */
        const frame = new St.Widget({
            reactive: false,
            can_focus: false,
            track_hover: false,
            layout_manager: new Clutter.BinLayout(),
            constraints: new Layout.MonitorConstraint({primary: true}),
        });
        const card = new St.Widget({
            name: 'soos-lockscreen',
            style_class: 'soos-lock-card',
            reactive: false,
            can_focus: false,
            track_hover: false,
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.START,
            layout_manager: new Clutter.BoxLayout({
                orientation: Clutter.Orientation.HORIZONTAL,
                spacing: 12,
            }),
        });
        const ring = new St.Widget({
            style_class: 'soos-lock-ring soos-idle',
            reactive: false,
            can_focus: false,
            track_hover: false,
            y_align: Clutter.ActorAlign.CENTER,
            layout_manager: new Clutter.BinLayout(),
        });
        const star = new St.DrawingArea({
            style_class: 'soos-lock-star',
            reactive: false,
            can_focus: false,
            track_hover: false,
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
        });
        star.set_pivot_point(0.5, 0.5);
        star.connect('repaint', area => {
            try {
                drawStar(area);
            } catch {
                console.warn('soos-lockscreen: drawing the star failed');
            }
        });
        const label = new St.Label({
            style_class: 'soos-lock-label',
            text: '',
            visible: false,
            reactive: false,
            can_focus: false,
            track_hover: false,
            y_align: Clutter.ActorAlign.CENTER,
        });
        ring.add_child(star);
        card.add_child(ring);
        card.add_child(label);
        frame.add_child(card);
        this._frame = frame;
        this._card = card;
        this._ring = ring;
        this._star = star;
        this._label = label;
        this._starAngle = 0;
        this._applyMotionClass();
        /* Last child of the shield group: drawn above the shell's own lock screen content. */
        Main.layoutManager.screenShieldGroup.add_child(frame);
        card.opacity = 0;
        card.ease({
            opacity: 255,
            duration: T_SHORT_MS,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
        });
        this._applyView(IDLE_VIEW);
    }

    _applyMotionClass() {
        if (this._card === null)
            return;
        if (this._reducedMotion())
            this._card.add_style_class_name('soos-reduced-motion');
        else
            this._card.remove_style_class_name('soos-reduced-motion');
    }

    _applyView(view) {
        this._view = view;
        if (this._card === null)
            return;
        const shown = present(view);
        for (const state of STATES)
            this._ring.remove_style_class_name(RING_CLASSES[state]);
        this._ring.add_style_class_name(RING_CLASSES[shown]);
        this._label.text = LABELS[shown];
        this._label.visible = shown !== 'idle';
        if (shown === 'scanning') {
            const now = nowMs();
            if (view.sequence !== this._scanSequence || now - this._scanStartMs > SCAN_MAX_AGE_MS) {
                this._scanSequence = view.sequence;
                this._scanStartMs = now;
            }
            this._startTurning();
        }
        if (shown === 'not_recognized' && view.sequence !== 0 &&
            view.sequence !== this._shakenSequence) {
            this._shakenSequence = view.sequence;
            this._shake();
        }
    }

    /* One quarter turn per T_PENDING_MS while scanning, bounded by PENDING_LOOP_MAX_MS. */
    _startTurning() {
        if (this._turnId !== 0 || this._reducedMotion())
            return;
        this._stepStar();
        this._turnId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, T_PENDING_MS, () => {
            try {
                return this._onTurnTick();
            } catch {
                this._turnId = 0;
                return GLib.SOURCE_REMOVE;
            }
        });
    }

    _onTurnTick() {
        try {
            if (this._star === null) {
                this._turnId = 0;
                return GLib.SOURCE_REMOVE;
            }
            const turning = present(this._view) === 'scanning' && !this._reducedMotion() &&
                nowMs() - this._scanStartMs < PENDING_LOOP_MAX_MS;
            /* The mark repeats every half turn: always come to rest on one. */
            if (turning || this._starAngle % 180 !== 0) {
                this._stepStar();
                return GLib.SOURCE_CONTINUE;
            }
            this._turnId = 0;
            return GLib.SOURCE_REMOVE;
        } catch {
            this._turnId = 0;
            return GLib.SOURCE_REMOVE;
        }
    }

    _stepStar() {
        this._starAngle += 90;
        this._star.ease({
            rotation_angle_z: this._starAngle,
            duration: T_PENDING_MS,
            mode: Clutter.AnimationMode.EASE_OUT_CUBIC,
        });
    }

    /* One short horizontal shake (at most SLIDE_PX) over T_MEDIUM_MS per new failure. */
    _shake() {
        if (this._reducedMotion())
            return;
        this._shakeLeg([SLIDE_PX, -SLIDE_PX, SLIDE_PX / 2, 0], 0);
    }

    _shakeLeg(legs, index) {
        if (this._card === null || index >= legs.length)
            return;
        this._card.ease({
            translation_x: legs[index],
            duration: T_MEDIUM_MS / legs.length,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            onComplete: () => {
                try {
                    this._shakeLeg(legs, index + 1);
                } catch {
                    console.warn('soos-lockscreen: shake failed');
                }
            },
        });
    }

    _startBridge() {
        if (this._bridge !== null || this._card === null)
            return;
        const path = findLockstatus();
        if (path === null) {
            console.warn('soos-lockscreen: no root-owned soos-lockstatus in /usr/bin or /usr/local/bin');
            return;
        }
        let proc;
        try {
            proc = Gio.Subprocess.new([path, '--watch'],
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_SILENCE);
        } catch {
            console.warn('soos-lockscreen: starting soos-lockstatus failed');
            this._scheduleRespawn();
            return;
        }
        const bridge = {
            proc,
            stdout: proc.get_stdout_pipe(),
            cancellable: new Gio.Cancellable(),
            pending: new Uint8Array(0),
            decoder: new TextDecoder('utf-8', {fatal: true}),
        };
        this._bridge = bridge;
        proc.wait_async(bridge.cancellable, (source, result) => {
            try {
                this._onBridgeExited(bridge, source, result);
            } catch {
                this._bridgeFailed(bridge);
            }
        });
        this._readNext(bridge);
    }

    _readNext(bridge) {
        bridge.stdout.read_bytes_async(READ_CHUNK_BYTES, GLib.PRIORITY_DEFAULT,
            bridge.cancellable, (stream, result) => {
                try {
                    this._onBridgeBytes(bridge, stream, result);
                } catch {
                    console.warn('soos-lockscreen: reading soos-lockstatus failed');
                    this._bridgeFailed(bridge);
                }
            });
    }

    _onBridgeBytes(bridge, stream, result) {
        try {
            let bytes;
            try {
                bytes = stream.read_bytes_finish(result);
            } catch {
                this._bridgeFailed(bridge);
                return;
            }
            if (bridge !== this._bridge)
                return;
            const data = bytes.get_data();
            if (data === null || data.length === 0) {
                this._bridgeFailed(bridge);
                return;
            }
            if (!this._consume(bridge, data)) {
                this._bridgeFailed(bridge);
                return;
            }
            this._readNext(bridge);
        } catch {
            console.warn('soos-lockscreen: reading soos-lockstatus failed');
            this._bridgeFailed(bridge);
        }
    }

    _onBridgeExited(bridge, proc, result) {
        try {
            proc.wait_finish(result);
        } catch {
            /* Cancelled by _stopBridge, or the wait failed: either way the bridge is gone. */
        }
        this._bridgeFailed(bridge);
    }

    /*
     * Appends bytes and applies every complete line; false on a line longer than MAX_LINE_BYTES.
     * The pending buffer never exceeds MAX_LINE_BYTES + READ_CHUNK_BYTES.
     */
    _consume(bridge, data) {
        const joined = new Uint8Array(bridge.pending.length + data.length);
        joined.set(bridge.pending, 0);
        joined.set(data, bridge.pending.length);
        let start = 0;
        let end = joined.indexOf(10, start);
        while (end !== -1) {
            if (end - start > MAX_LINE_BYTES)
                return false;
            let view = null;
            try {
                view = parseLine(bridge.decoder.decode(joined.subarray(start, end)));
            } catch {
                view = null;
            }
            if (view !== null) {
                this._respawnIndex = 0;
                this._applyView(view);
            }
            start = end + 1;
            end = joined.indexOf(10, start);
        }
        bridge.pending = joined.slice(start);
        return bridge.pending.length <= MAX_LINE_BYTES;
    }

    /* The bridge ended or misbehaved: draw Idle and restart it later while still locked. */
    _bridgeFailed(bridge) {
        if (bridge !== this._bridge)
            return;
        this._stopBridge();
        this._applyView(IDLE_VIEW);
        this._scheduleRespawn();
    }

    _scheduleRespawn() {
        if (this._respawnId !== 0 || this._card === null)
            return;
        const index = Math.min(this._respawnIndex, RESPAWN_DELAYS_MS.length - 1);
        this._respawnIndex = Math.min(this._respawnIndex + 1, RESPAWN_DELAYS_MS.length);
        this._respawnId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, RESPAWN_DELAYS_MS[index], () => {
            try {
                return this._onRespawn();
            } catch {
                this._respawnId = 0;
                return GLib.SOURCE_REMOVE;
            }
        });
    }

    _onRespawn() {
        try {
            this._respawnId = 0;
            if (this._card !== null && this._bridge === null)
                this._startBridge();
        } catch {
            console.warn('soos-lockscreen: restarting soos-lockstatus failed');
        }
        return GLib.SOURCE_REMOVE;
    }

    _stopBridge() {
        const bridge = this._bridge;
        this._bridge = null;
        if (!bridge)
            return;
        try {
            bridge.cancellable.cancel();
        } catch {
            console.warn('soos-lockscreen: cancelling the bridge read failed');
        }
        try {
            bridge.proc.force_exit();
        } catch {
            console.warn('soos-lockscreen: stopping soos-lockstatus failed');
        }
    }

    _removeSources() {
        for (const key of ['_turnId', '_respawnId']) {
            try {
                if (this[key])
                    GLib.Source.remove(this[key]);
            } catch {
                console.warn('soos-lockscreen: removing a timer failed');
            }
            this[key] = 0;
        }
    }

    _destroyCard() {
        const frame = this._frame;
        this._frame = null;
        this._card = null;
        this._ring = null;
        this._star = null;
        this._label = null;
        this._starAngle = 0;
        this._scanSequence = -1;
        this._shakenSequence = -1;
        try {
            if (frame)
                frame.destroy();
        } catch {
            console.warn('soos-lockscreen: removing the card failed');
        }
    }

    _stopAll() {
        this._removeSources();
        this._stopBridge();
        this._destroyCard();
        this._respawnIndex = 0;
        this._view = IDLE_VIEW;
    }
}
