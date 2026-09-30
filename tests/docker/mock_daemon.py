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

Usage:
  python3 mock_daemon.py --socket /run/soos/daemon.sock --mode allow [--delay 0.5] [--one-shot]
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
    parser.add_argument("--socket-group", default="soos", help="Group owning the socket (default: soos)")
    parser.add_argument(
        "--socket-mode",
        default="0660",
        help="Octal socket mode (default: 0660); any 'other' permission bit is refused",
    )
    args = parser.parse_args()

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
