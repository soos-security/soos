#!/usr/bin/env python3
"""
mock_daemon.py — Lightweight Unix Domain Socket Mock Daemon for PAM Testing

Simulates daemon behaviors for the PAM test matrix:
  - allow:           Returns Verdict::Allow immediately
  - deny:            Returns Verdict::Deny immediately
  - timeout:         Sleeps for --delay seconds (default 0.5s) before responding or dropping
  - crash-immediate: Closes stream immediately after accept
  - crash-partial:   Sends partial length header (2 bytes) and closes stream
  - crash-truncated: Sends length header and truncated body (4 bytes) and closes stream
  - malformed:       Complete frame, echoed request_id, undecodable verdict discriminant
  - wrong-request-id: Complete Allow frame whose request_id does not match the request
  - bad-version:     Complete Allow frame with protocol version 2
  - oversized:       Length prefix above MAX_MESSAGE_SIZE (4096), no body, then close
  - empty:           Zero length prefix, then close
  - expired:         Complete Allow frame whose issued/expires window already closed

The malformed modes (GitHub #189) each put exactly one defect on the wire; three of
them carry an Allow verdict that the PAM module must never honor.

Response stamps (GitHub #287): pam_soos.so rejects a Response whose CLOCK_MONOTONIC
issued/expires stamps are unset, inconsistent, future-dated or expired. By default
(`--stamps monotonic`) every response is stamped like soos-daemon (issued = now,
expires = now + 2 s, read when the response is sent), so each malformed mode keeps
exactly one defect; the scripts also pass the flag explicitly. `--stamps zero` sends
the unstamped form 0/0, which the module rejects (Docker case T15).

Every received Event (e.g. PasswordFailed) is recorded with --record and never
answered; Requests are answered according to --mode.

Usage:
  python3 mock_daemon.py --socket /run/soos/daemon.sock --mode allow [--stamps monotonic|zero]
                         [--delay 0.5] [--one-shot]
                         [--record /tmp/events.log]
                         [--socket-group soos] [--socket-mode 0660]

The socket mirrors the production invariant (ARCHITECTURE.md, GitHub #168): it is
created with mode 0660 and group-owned by --socket-group (default: soos). Modes that
grant any permission to "other" are refused, and a missing group is a hard error
(fail closed, never a silent fallback to a world-accessible socket).
"""

import argparse
import grp
import os
import signal
import socket
import struct
import sys
import time

CURRENT_VERSION = 1
VERDICT_ALLOW = 0
VERDICT_DENY = 1
VERDICT_UNAVAILABLE = 2
REASON_FACEMATCH = 0
# ReasonClass wire index 3 (index 1 is NoFace); user-approved fix 2026-09-30, GitHub #226.
REASON_SCORE_BELOW_THRESHOLD = 3


VERDICT_UNDECODABLE = 0x7F  # single-byte varint outside the Verdict enum
UNSUPPORTED_VERSION = 2
MAX_MESSAGE_SIZE = 4096


# soos-daemon RESPONSE_VALIDITY_NS (crates/daemon/src/dispatcher.rs): expires = issued + 2 s.
RESPONSE_VALIDITY_NS = 2_000_000_000
# How far in the past the window of an `expired` response closed.
EXPIRED_AGE_NS = 1_000_000_000


def monotonic_ns() -> int:
    """CLOCK_MONOTONIC in nanoseconds: the clock soos-daemon stamps with and pam_soos.so reads."""
    return time.clock_gettime_ns(time.CLOCK_MONOTONIC)


def write_varint(out: bytearray, value: int):
    """Appends a postcard (LEB128) unsigned varint; value must fit a u64."""
    if value < 0 or value >= 1 << 64:
        raise ValueError("varint out of u64 range")
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return


def response_stamps(stamps: str, mode: str):
    """Returns (issued_monotonic_ns, expires_monotonic_ns) for one response, read at send time.

    `monotonic` stamps exactly like soos-daemon (issued = now, expires = issued + 2 s), which
    the PAM client enforces since GitHub #287. `zero` sends the unstamped form (0, 0) that a
    daemon without a clock would send; the PAM client rejects it. The `expired` mode always
    sends a consistent window that closed EXPIRED_AGE_NS before now.
    """
    if mode == "expired":
        expires = max(monotonic_ns() - EXPIRED_AGE_NS, 2)
        return max(expires - RESPONSE_VALIDITY_NS, 1), expires
    if stamps == "monotonic":
        issued = monotonic_ns()
        return issued, issued + RESPONSE_VALIDITY_NS
    return 0, 0


def build_response(
    request_id: bytes, verdict: int, reason: int, version: int = CURRENT_VERSION, stamps=(0, 0)
) -> bytes:
    """Builds a framed postcard-compatible Response wire payload."""
    # Wire layout:
    # version (u8) = 1
    # request_id ([u8; 32])
    # verdict (varint u8)
    # reason_class (varint u8)
    # issued_monotonic_ns (varint u64)
    # expires_monotonic_ns (varint u64)
    issued, expires = stamps
    body = bytearray()
    body.append(version)
    body.extend(request_id)
    body.append(verdict)
    body.append(reason)
    write_varint(body, issued)
    write_varint(body, expires)

    length_prefix = struct.pack(">I", len(body))
    return length_prefix + bytes(body)


def read_varint(buf: bytes, idx: int):
    """Decodes a postcard (LEB128) unsigned varint starting at idx."""
    value = 0
    shift = 0
    while True:
        byte = buf[idx]
        idx += 1
        value |= (byte & 0x7F) << shift
        if byte & 0x80 == 0:
            return value, idx
        shift += 7
        if shift > 63:
            raise ValueError("varint too long")


def classify_event(body: bytes):
    """Returns (kind, uid, service) if body is a complete postcard Event, else None.

    Event wire layout: version (u8), kind (varint), request_id (Option<[u8; 32]>),
    uid (Option<varint u32>), service (varint length + UTF-8), timestamp (varint).
    A Request never decodes to exactly its own length under this layout.
    """
    try:
        if len(body) < 6 or body[0] != CURRENT_VERSION:
            return None
        kind, idx = read_varint(body, 1)
        tag = body[idx]
        idx += 1
        if tag == 1:
            idx += 32
        elif tag != 0:
            return None
        tag = body[idx]
        idx += 1
        uid = None
        if tag == 1:
            uid, idx = read_varint(body, idx)
        elif tag != 0:
            return None
        service_len, idx = read_varint(body, idx)
        if idx + service_len > len(body):
            return None
        service = bytes(body[idx:idx + service_len]).decode("utf-8")
        idx += service_len
        _, idx = read_varint(body, idx)
        if idx != len(body):
            return None
        return kind, uid, service
    except (IndexError, ValueError, UnicodeDecodeError):
        return None


EVENT_KIND_NAMES = {0: "password-failed"}

# Client frame tag trailers (crates/protocol/src/message.rs, GitHub #204).
MESSAGE_TAG_REQUEST = 0xA0
MESSAGE_TAG_EVENT = 0xA1
MESSAGE_TAG_MIN = 0x80


def record_line(path, line: str):
    """Appends one line (message classification only, never payload bytes)."""
    if path:
        with open(path, "a", encoding="utf-8") as handle:
            handle.write(line + "\n")


def main():
    parser = argparse.ArgumentParser(description="soos test mock daemon")
    parser.add_argument("--socket", default="/run/soos/daemon.sock", help="Socket path")
    parser.add_argument(
        "--mode",
        choices=[
            "allow",
            "deny",
            "timeout",
            "crash-immediate",
            "crash-partial",
            "crash-truncated",
            "malformed",
            "wrong-request-id",
            "bad-version",
            "oversized",
            "empty",
            "expired",
        ],
        default="allow",
        help="Simulation behavior mode",
    )
    parser.add_argument(
        "--stamps",
        choices=["zero", "monotonic"],
        default="monotonic",
        help=(
            "Response issued/expires stamps: 'monotonic' (default) stamps like soos-daemon; "
            "'zero' sends the unstamped form (0, 0) that the PAM module rejects"
        ),
    )
    parser.add_argument("--delay", type=float, default=0.5, help="Delay in seconds for timeout mode")
    parser.add_argument("--one-shot", action="store_true", help="Exit after handling one connection")
    parser.add_argument(
        "--record",
        default=None,
        help="Append one line per received message ('event kind=... service=...' or 'request')",
    )
    parser.add_argument("--socket-group", default="soos", help="Group owning the socket (default: soos)")
    parser.add_argument(
        "--socket-mode",
        default="0660",
        help="Octal socket mode (default: 0660); any 'other' permission bit is refused",
    )
    args = parser.parse_args()

    def send_stamps():
        """Stamps read when the response is built (after any `timeout` delay)."""
        return response_stamps(args.stamps, args.mode)

    try:
        socket_mode = int(args.socket_mode, 8)
    except ValueError:
        parser.error(f"--socket-mode must be an octal mode, got '{args.socket_mode}'")
    if socket_mode & 0o007 or socket_mode & ~0o770:
        parser.error(
            f"--socket-mode {args.socket_mode} is refused: the daemon socket must never be "
            "accessible to 'other' (production invariant: 0660 root:soos)"
        )
    try:
        socket_gid = grp.getgrnam(args.socket_group).gr_gid
    except KeyError:
        parser.error(f"--socket-group '{args.socket_group}' does not exist (create it with groupadd -r)")

    sock_path = args.socket
    os.makedirs(os.path.dirname(sock_path), exist_ok=True)
    if os.path.exists(sock_path):
        try:
            os.unlink(sock_path)
        except OSError:
            pass

    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)

    def cleanup(*_):
        try:
            server.close()
        except Exception:
            pass
        if os.path.exists(sock_path):
            try:
                os.unlink(sock_path)
            except OSError:
                pass
        sys.exit(0)

    signal.signal(signal.SIGINT, cleanup)
    signal.signal(signal.SIGTERM, cleanup)

    # Bind under a restrictive umask so the socket is never briefly world-accessible,
    # then apply the exact mode and group (the owner stays the invoking user: root in
    # the containers, matching the production root:soos ownership).
    previous_umask = os.umask(0o117)
    try:
        server.bind(sock_path)
    finally:
        os.umask(previous_umask)
    os.chown(sock_path, -1, socket_gid)
    os.chmod(sock_path, socket_mode)
    server.listen(16)
    print(f"[mock_daemon] Listening on {sock_path} in mode '{args.mode}'", flush=True)

    try:
        while True:
            client, _ = server.accept()
            try:
                if args.mode == "crash-immediate":
                    # Accept and immediately close
                    client.close()
                    if args.one_shot:
                        break
                    continue

                # Read 4-byte length prefix
                len_bytes = client.recv(4)
                if len(len_bytes) < 4:
                    client.close()
                    continue
                body_len = struct.unpack(">I", len_bytes)[0]
                body = bytearray()
                while len(body) < body_len:
                    chunk = client.recv(min(4096, body_len - len(body)))
                    if not chunk:
                        break
                    body.extend(chunk)

                # Codec v1 client frames carry a one-byte message tag trailer (GitHub #204,
                # crates/protocol/src/message.rs): 0xA0 = Request, 0xA1 = Event. Strip it so
                # the body decodes as before; untagged legacy frames are still classified.
                frame_tag = None
                if body and body[-1] >= MESSAGE_TAG_MIN:
                    frame_tag = body[-1]
                    body = body[:-1]
                    # Parity with decode_client_message (MessageError::UnknownTag, GitHub #285):
                    # an unknown tag runs no handler and gets no response.
                    if frame_tag not in (MESSAGE_TAG_REQUEST, MESSAGE_TAG_EVENT):
                        record_line(args.record, "rejected unknown-tag")
                        continue

                # Events are fire-and-forget: record them and never reply.
                event = classify_event(bytes(body)) if frame_tag != MESSAGE_TAG_REQUEST else None
                if event is not None:
                    kind, uid, service = event
                    kind_name = EVENT_KIND_NAMES.get(kind, f"unknown-{kind}")
                    record_line(args.record, f"event kind={kind_name} uid={uid} service={service}")
                    continue
                record_line(args.record, "request")

                # Extract request_id from Request body (bytes 2..34)
                req_id = bytes(body[2:34]) if len(body) >= 34 else b"\x00" * 32

                if args.mode == "timeout":
                    time.sleep(args.delay)
                    # After sleeping beyond timeout, send allow
                    resp = build_response(req_id, VERDICT_ALLOW, REASON_FACEMATCH, stamps=send_stamps())
                    client.sendall(resp)
                elif args.mode == "crash-partial":
                    # Send partial 2 bytes of length prefix then close abruptly
                    client.sendall(b"\x00\x00")
                elif args.mode == "crash-truncated":
                    # Send length prefix claiming 37 bytes, but send only 4 bytes of body
                    client.sendall(struct.pack(">I", 37) + b"\x01\xAA\xBB\xCC")
                elif args.mode == "malformed":
                    resp = build_response(req_id, VERDICT_UNDECODABLE, REASON_FACEMATCH, stamps=send_stamps())
                    client.sendall(resp)
                elif args.mode == "wrong-request-id":
                    other_id = bytes(b ^ 0xFF for b in req_id)
                    resp = build_response(other_id, VERDICT_ALLOW, REASON_FACEMATCH, stamps=send_stamps())
                    client.sendall(resp)
                elif args.mode == "bad-version":
                    resp = build_response(
                        req_id, VERDICT_ALLOW, REASON_FACEMATCH, UNSUPPORTED_VERSION,
                        stamps=send_stamps(),
                    )
                    client.sendall(resp)
                elif args.mode == "oversized":
                    client.sendall(struct.pack(">I", MAX_MESSAGE_SIZE + 1))
                elif args.mode == "empty":
                    client.sendall(struct.pack(">I", 0))
                elif args.mode == "deny":
                    resp = build_response(req_id, VERDICT_DENY, REASON_SCORE_BELOW_THRESHOLD, stamps=send_stamps())
                    client.sendall(resp)
                else:  # allow, expired
                    resp = build_response(req_id, VERDICT_ALLOW, REASON_FACEMATCH, stamps=send_stamps())
                    client.sendall(resp)
            except Exception as e:
                print(f"[mock_daemon] Connection handling error: {e}", file=sys.stderr, flush=True)
            finally:
                try:
                    client.close()
                except Exception:
                    pass

            if args.one_shot:
                break
    finally:
        cleanup()


if __name__ == "__main__":
    main()
