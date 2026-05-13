"""Client helpers for talking to the socai tool daemon."""

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import time
import uuid
from pathlib import Path
from typing import Any

from socai.cli.daemon import PID_PATH, SOCAI_HOME, SOCKET_PATH


class DaemonError(RuntimeError):
    """Raised when the daemon returns ok=false or the transport fails."""


def _connect(timeout: float = 2.0) -> socket.socket | None:
    if not SOCKET_PATH.exists():
        return None
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(timeout)
    try:
        sock.connect(str(SOCKET_PATH))
    except (FileNotFoundError, ConnectionRefusedError, OSError):
        try:
            sock.close()
        except Exception:
            pass
        return None
    return sock


def is_running() -> bool:
    sock = _connect(timeout=0.5)
    if sock is None:
        return False
    try:
        return _send_on_socket(sock, "ping", {}, request_timeout=2.0).get("ok", False)
    except Exception:
        return False
    finally:
        try:
            sock.close()
        except Exception:
            pass


def spawn_daemon(*, wait_seconds: float = 30.0) -> None:
    """Spawn the daemon as a detached background process and wait for it."""
    SOCAI_HOME.mkdir(parents=True, exist_ok=True)
    if is_running():
        return

    # Detach: new session, redirect stdio to log file. The daemon writes its
    # own structured log via ``_log``, so plain stdio goes to the same file
    # for any stray prints.
    log_fh = open(SOCAI_HOME / "daemon.log", "a", encoding="utf-8")
    log_fh.write(f"\n--- daemon spawn @ {time.strftime('%Y-%m-%d %H:%M:%S')} ---\n")
    log_fh.flush()
    subprocess.Popen(
        [sys.executable, "-m", "socai.cli.daemon"],
        stdin=subprocess.DEVNULL,
        stdout=log_fh,
        stderr=log_fh,
        start_new_session=True,
        close_fds=True,
    )

    deadline = time.time() + wait_seconds
    while time.time() < deadline:
        if is_running():
            return
        time.sleep(0.2)
    raise DaemonError(
        f"daemon failed to start within {wait_seconds:.0f}s. Check {SOCAI_HOME / 'daemon.log'}"
    )


def _send_on_socket(
    sock: socket.socket,
    cmd: str,
    args: dict[str, Any],
    *,
    request_timeout: float,
) -> dict[str, Any]:
    req_id = uuid.uuid4().hex
    payload = json.dumps({"id": req_id, "cmd": cmd, "args": args}) + "\n"
    sock.settimeout(request_timeout)
    sock.sendall(payload.encode("utf-8"))

    buf = bytearray()
    while True:
        chunk = sock.recv(65536)
        if not chunk:
            raise DaemonError("daemon closed connection before responding")
        buf.extend(chunk)
        if b"\n" in chunk:
            break
    line = bytes(buf).split(b"\n", 1)[0]
    try:
        return json.loads(line.decode("utf-8"))
    except json.JSONDecodeError as exc:
        raise DaemonError(f"daemon returned malformed JSON: {exc}") from exc


def send(
    cmd: str,
    args: dict[str, Any] | None = None,
    *,
    request_timeout: float = 600.0,
    auto_spawn: bool = True,
) -> dict[str, Any]:
    """Send one command to the daemon and return its parsed response.

    Auto-spawns the daemon if it isn't running. ``request_timeout`` covers the
    whole request including in-browser work — topic_scan can take a while.
    """
    sock = _connect(timeout=2.0)
    if sock is None:
        if not auto_spawn:
            raise DaemonError("daemon is not running")
        spawn_daemon()
        sock = _connect(timeout=2.0)
        if sock is None:
            raise DaemonError("failed to connect to daemon after spawn")
    try:
        response = _send_on_socket(sock, cmd, args or {}, request_timeout=request_timeout)
    finally:
        try:
            sock.close()
        except Exception:
            pass

    if not response.get("ok", False):
        err = response.get("error") or "unknown daemon error"
        tb = response.get("traceback")
        if tb:
            sys.stderr.write(tb + "\n")
        raise DaemonError(err)
    return response.get("result") or {}


def stop_daemon(*, timeout: float = 5.0) -> bool:
    """Ask the daemon to shut down. Returns True if it was running."""
    if not is_running():
        return False
    try:
        send("shutdown", {}, request_timeout=timeout, auto_spawn=False)
    except DaemonError:
        pass
    # Wait for socket cleanup as a liveness signal.
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not SOCKET_PATH.exists():
            return True
        time.sleep(0.1)
    return not is_running()


def pid_from_file() -> int | None:
    try:
        return int(Path(PID_PATH).read_text(encoding="utf-8").strip())
    except Exception:
        return None
