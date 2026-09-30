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

Every received Event (e.g. PasswordFailed) is recorded with --record and never
answered; Requests are answered according to --mode.

Usage:
  python3 mock_daemon.py --socket /run/soos/daemon.sock --mode allow [--delay 0.5] [--one-shot]
                         [--record /tmp/events.log]
"""

import argparse
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
REASON_SCORE_BELOW_THRESHOLD = 1


def build_response(request_id: bytes, verdict: int, reason: int) -> bytes:
    """Builds a framed postcard-compatible Response wire payload."""
    # Wire layout:
    # version (u8) = 1
    # request_id ([u8; 32])
    # verdict (varint u8)
    # reason_class (varint u8)
    # issued_monotonic_ns (varint = 0)
    # expires_monotonic_ns (varint = 0)
    body = bytearray()
    body.append(CURRENT_VERSION)
    body.extend(request_id)
    body.append(verdict)
    body.append(reason)
    body.append(0)  # issued_monotonic_ns = 0
    body.append(0)  # expires_monotonic_ns = 0

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
        choices=["allow", "deny", "timeout", "crash-immediate", "crash-partial", "crash-truncated"],
        default="allow",
        help="Simulation behavior mode",
    )
    parser.add_argument("--delay", type=float, default=0.5, help="Delay in seconds for timeout mode")
    parser.add_argument("--one-shot", action="store_true", help="Exit after handling one connection")
    parser.add_argument(
        "--record",
        default=None,
        help="Append one line per received message ('event kind=... service=...' or 'request')",
    )
    args = parser.parse_args()

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

    server.bind(sock_path)
    os.chmod(sock_path, 0o666)  # allow testuser and root access in test container
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

                # Events are fire-and-forget: record them and never reply.
                event = classify_event(bytes(body))
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
                    resp = build_response(req_id, VERDICT_ALLOW, REASON_FACEMATCH)
                    client.sendall(resp)
                elif args.mode == "crash-partial":
                    # Send partial 2 bytes of length prefix then close abruptly
                    client.sendall(b"\x00\x00")
                elif args.mode == "crash-truncated":
                    # Send length prefix claiming 37 bytes, but send only 4 bytes of body
                    client.sendall(struct.pack(">I", 37) + b"\x01\xAA\xBB\xCC")
                elif args.mode == "deny":
                    resp = build_response(req_id, VERDICT_DENY, REASON_SCORE_BELOW_THRESHOLD)
                    client.sendall(resp)
                else:  # allow
                    resp = build_response(req_id, VERDICT_ALLOW, REASON_FACEMATCH)
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
