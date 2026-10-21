#!/usr/bin/env python3
"""Drive the real Scratch app in a headless Flutter test process (stdlib only)."""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request

APP = Path(__file__).resolve().parents[1]
BUILD = APP / "build" / "drive"
SESSION = BUILD / "session.json"
OWNER = BUILD / "owner.json"
WINDOWS = os.name == "nt"


class DriveError(Exception):
    pass


def hidden_flags() -> int:
    return subprocess.CREATE_NO_WINDOW if WINDOWS else 0


def read_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, ValueError) as exc:
        raise DriveError(f"cannot read {path.name}: {exc}") from exc


def write_owner(data: dict) -> None:
    tmp = OWNER.with_suffix(".tmp")
    tmp.write_text(json.dumps(data), encoding="utf-8")
    tmp.replace(OWNER)


def processes() -> list[dict]:
    """Creation timestamps prevent an old owner file from killing a reused PID."""
    if WINDOWS:
        import ctypes as c
        from ctypes import wintypes as w

        class ProcessEntry(c.Structure):
            _fields_ = [("dwSize", w.DWORD), ("cntUsage", w.DWORD),
                        ("th32ProcessID", w.DWORD), ("th32DefaultHeapID", c.c_size_t),
                        ("th32ModuleID", w.DWORD), ("cntThreads", w.DWORD),
                        ("th32ParentProcessID", w.DWORD), ("pcPriClassBase", w.LONG),
                        ("dwFlags", w.DWORD), ("szExeFile", w.WCHAR * 260)]

        kernel = c.WinDLL("kernel32", use_last_error=True)
        kernel.CreateToolhelp32Snapshot.argtypes = [w.DWORD, w.DWORD]
        kernel.CreateToolhelp32Snapshot.restype = w.HANDLE
        kernel.Process32FirstW.argtypes = [w.HANDLE, c.POINTER(ProcessEntry)]
        kernel.Process32NextW.argtypes = [w.HANDLE, c.POINTER(ProcessEntry)]
        kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
        kernel.OpenProcess.restype = w.HANDLE
        kernel.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(w.FILETIME)] * 4
        kernel.CloseHandle.argtypes = [w.HANDLE]
        snapshot = kernel.CreateToolhelp32Snapshot(0x2, 0)  # TH32CS_SNAPPROCESS
        if snapshot == c.c_void_p(-1).value:
            raise DriveError(f"cannot inventory session processes: Windows error {c.get_last_error()}")
        result = []
        entry = ProcessEntry()
        entry.dwSize = c.sizeof(entry)
        try:
            found = kernel.Process32FirstW(snapshot, c.byref(entry))
            while found:
                handle = kernel.OpenProcess(0x1000, False, entry.th32ProcessID)
                created = None
                if handle:
                    try:
                        times = [w.FILETIME() for _ in range(4)]
                        if kernel.GetProcessTimes(handle, *(c.byref(t) for t in times)):
                            created = (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime
                    finally:
                        kernel.CloseHandle(handle)
                result.append(dict(ProcessId=entry.th32ProcessID,
                                   ParentProcessId=entry.th32ParentProcessID,
                                   Name=entry.szExeFile, Created=created))
                found = kernel.Process32NextW(snapshot, c.byref(entry))
        finally:
            kernel.CloseHandle(snapshot)
        return result
    # /proc is available on Linux; ps provides a portable fallback for macOS.
    result = subprocess.run(["ps", "-axo", "pid=,ppid=,lstart=,comm="],
                            capture_output=True, text=True, check=True)
    rows = []
    for line in result.stdout.splitlines():
        parts = line.split(None, 7)
        if len(parts) == 8:
            rows.append(dict(ProcessId=int(parts[0]), ParentProcessId=int(parts[1]),
                             Created=" ".join(parts[2:7]), Name=parts[7]))
    return rows


def signature(row: dict) -> dict:
    return {key: row[key] for key in ("ProcessId", "Created", "Name")}


def owned_processes(owner: dict, rows: list[dict]) -> list[dict]:
    saved = owner.get("processes", [])
    owned = {r["ProcessId"] for r in rows if signature(r) in saved}
    while True:
        children = {r["ProcessId"] for r in rows if r["ParentProcessId"] in owned}
        updated = owned | children
        if updated == owned:
            return [r for r in rows if r["ProcessId"] in owned]
        owned = updated


def kill_owned(owner: dict) -> list[dict]:
    rows = processes()
    owned = owned_processes(owner, rows)
    # Kill only recorded session members, rechecking identity before each kill.
    if WINDOWS:
        import ctypes as c
        from ctypes import wintypes as w
        kernel = c.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
        kernel.OpenProcess.restype = w.HANDLE
        kernel.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(w.FILETIME)] * 4
        kernel.TerminateProcess.argtypes = [w.HANDLE, w.UINT]
        kernel.CloseHandle.argtypes = [w.HANDLE]
        for row in owned:
            if not any(signature(r) == signature(row) for r in processes()):
                continue
            handle = kernel.OpenProcess(0x1001, False, row["ProcessId"])
            if not handle:
                if not any(signature(r) == signature(row) for r in processes()):
                    continue
                raise DriveError(f'cannot stop session process {row["ProcessId"]}: Windows error {c.get_last_error()}')
            try:
                times = [w.FILETIME() for _ in range(4)]
                if not kernel.GetProcessTimes(handle, *(c.byref(t) for t in times)):
                    raise DriveError(f'cannot verify session process {row["ProcessId"]}')
                created = (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime
                if created != row["Created"]:
                    continue
                if not kernel.TerminateProcess(handle, 1):
                    raise DriveError(f'cannot stop session process {row["ProcessId"]}: Windows error {c.get_last_error()}')
            finally:
                kernel.CloseHandle(handle)
    else:
        import signal
        for row in reversed(owned):
            try:
                os.kill(row["ProcessId"], signal.SIGKILL)
            except ProcessLookupError:
                pass
    deadline = time.monotonic() + 10
    while True:
        remaining = [r for r in processes() if signature(r) in [signature(p) for p in owned]]
        if not remaining or time.monotonic() >= deadline:
            return remaining
        time.sleep(0.1)


def request(command: dict, session: dict | None = None, timeout: float = 45) -> dict:
    session = session or read_json(SESSION)
    try:
        port = int(session["port"])
        token = session["token"]
        if not 1 <= port <= 65535 or not isinstance(token, str):
            raise ValueError("invalid port or token")
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}/command",
            data=json.dumps(command, ensure_ascii=False).encode("utf-8"),
            headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
            method="POST",
        )
        # Ignore proxy settings: this authenticated service is strictly loopback.
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req, timeout=timeout) as response:
            reply = json.load(response)
    except (OSError, ValueError, KeyError, urllib.error.URLError) as exc:
        raise DriveError(f"host unavailable: {exc}") from exc
    if not isinstance(reply, dict):
        raise DriveError("host returned an invalid reply")
    return reply


def size(value: str) -> tuple[int, int]:
    try:
        w, h = map(int, value.lower().split("x"))
        if not (1 <= w <= 10000 and 1 <= h <= 10000):
            raise ValueError()
        return w, h
    except ValueError as exc:
        raise argparse.ArgumentTypeError("size must be WxH, with positive dimensions up to 10000") from exc


def dimensions(args: argparse.Namespace) -> dict:
    mobile = bool(args.mobile)
    width, height = args.size or ((390, 844) if mobile else (1440, 900))
    return dict(mobile=mobile, width=width, height=height)


def start(args: argparse.Namespace) -> None:
    BUILD.mkdir(parents=True, exist_ok=True)
    if SESSION.exists():
        try:
            reply = request({"command": "ping"}, timeout=2)
            if reply.get("ok"):
                raise DriveError("a drive session is already running; use stop first")
        except DriveError as exc:
            if "already running" in str(exc):
                raise
        if OWNER.exists() and owned_processes(read_json(OWNER), processes()):
            raise DriveError("a session process is still running; use stop first")
        SESSION.unlink(missing_ok=True)
    if OWNER.exists() and owned_processes(read_json(OWNER), processes()):
        raise DriveError("a session is starting or still running; use stop first")
    flutter = shutil.which("flutter") or shutil.which("flutter.bat")
    if not flutter and WINDOWS:
        fallback = Path.home() / "flutter" / "bin" / "flutter.bat"
        if fallback.is_file():
            flutter = str(fallback)
    if not flutter:
        raise DriveError(f"Flutter was not found on PATH or at {fallback.parent}")
    lib = APP.parent / "target" / "release" / ("notes_bridge.dll" if WINDOWS else
                                              "libnotes_bridge.dylib" if sys.platform == "darwin" else "libnotes_bridge.so")
    # A worktree without its own Rust build can point at another checkout's.
    if os.environ.get("NOTES_BRIDGE_LIB"):
        lib = Path(os.environ["NOTES_BRIDGE_LIB"])
    if not lib.is_file():
        raise DriveError(f"bridge library missing: {lib}; build with cargo build -p notes_bridge --release")
    if args.vault.startswith("copy:"):
        source = Path(args.vault[5:]).expanduser().resolve()
        if not source.is_dir():
            raise DriveError(f"vault source is not a folder: {source}")
        args.vault = "copy:" + str(source)
    elif args.vault not in ("fresh", "demo", "empty"):
        raise DriveError("vault must be fresh, demo, empty, or copy:<path>")
    if args.now:
        try:
            dt.datetime.strptime(args.now, "%Y-%m-%d %H:%M")
        except ValueError as exc:
            raise DriveError("now must be YYYY-MM-DD HH:MM") from exc
    env = os.environ.copy()
    env["NOTES_BRIDGE_LIB"] = str(lib.resolve())
    env["FLUTTER_SUPPRESS_ANALYTICS"] = "true"
    env["DART_SUPPRESS_ANALYTICS"] = "true"
    if WINDOWS:
        if not env.get("PUB_CACHE") and env.get("LOCALAPPDATA"):
            env["PUB_CACHE"] = str(Path(env["LOCALAPPDATA"]) / "Pub" / "Cache")
        for key, folder in (("APPDATA", "tool-home"), ("LOCALAPPDATA", "tool-local")):
            tool_home = BUILD / folder
            tool_home.mkdir(parents=True, exist_ok=True)
            env[key] = str(tool_home)
    config = dict(**dimensions(args), dark=args.dark, vault=args.vault)
    if args.now:
        config["now"] = args.now
    env["NOTES_DRIVE_CONFIG"] = json.dumps(config)
    flutter_command = [flutter]
    if WINDOWS:
        flutter_root = Path(flutter).resolve().parent.parent
        dart = flutter_root / "bin" / "cache" / "dart-sdk" / "bin" / "dart.exe"
        snapshot = flutter_root / "bin" / "cache" / "flutter_tools.snapshot"
        packages = flutter_root / "packages" / "flutter_tools" / ".dart_tool" / "package_config.json"
        if dart.is_file() and snapshot.is_file() and packages.is_file():
            # flutter.bat performs SDK bootstrap bookkeeping, which can hang in
            # a sandbox. This runs its existing Flutter tool snapshot directly.
            env["FLUTTER_ROOT"] = str(flutter_root)
            env["FLUTTER_ALREADY_LOCKED"] = "true"
            flutter_command = [str(dart), "--disable-dart-dev", f"--packages={packages}", str(snapshot)]
    cmd = flutter_command + ["test", "test/drive/drive_session_test.dart", "--tags", "drive",
                             "--reporter", "expanded", "--timeout", "none", "--no-pub"]
    with (BUILD / "host.log").open("wb") as output:
        host = subprocess.Popen(cmd, cwd=APP, env=env, stdin=subprocess.DEVNULL,
                                stdout=output, stderr=subprocess.STDOUT,
                                creationflags=hidden_flags(), start_new_session=not WINDOWS)
    owner = dict(launcherPid=host.pid, processes=[])
    try:
        rows = processes()
        owner["processes"] = [signature(r) for r in rows if r["ProcessId"] == host.pid]
        if not owner["processes"]:
            raise DriveError(f"launcher exited before it could be tracked; see {BUILD / 'host.log'}")
        if owner["processes"][0]["Created"] is None:
            raise DriveError("cannot verify launcher creation time")
        write_owner(owner)
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            if host.poll() is not None:
                raise DriveError(f"host exited with code {host.returncode}; see {BUILD / 'host.log'}")
            if SESSION.exists():
                try:
                    reply = request({"command": "ping"}, timeout=2)
                    if reply.get("ok"):
                        rows = processes()
                        owner["processes"] = [signature(r) for r in owned_processes(owner, rows)]
                        host_pid = int(read_json(SESSION)["pid"])
                        # The host PID must be a descendant of the launcher.
                        if not any(r["ProcessId"] == host_pid for r in owner["processes"]):
                            raise DriveError("session host is not owned by this launcher")
                        write_owner(owner)
                        print("ready")
                        return
                except DriveError as exc:
                    if "not owned" in str(exc):
                        raise
            time.sleep(0.1)
        raise DriveError(f"start timed out after 180 s; see {BUILD / 'host.log'}")
    except BaseException:
        if owner["processes"]:
            kill_owned(owner)
        elif host.poll() is None:
            # Popen retains the exact process handle, safe even on inventory failure.
            host.kill()
            host.wait(timeout=10)
        SESSION.unlink(missing_ok=True)
        OWNER.unlink(missing_ok=True)
        raise


def stop() -> dict:
    owner = read_json(OWNER) if OWNER.exists() else {"processes": []}
    rows = processes()
    owner["processes"] = [signature(r) for r in owned_processes(owner, rows)]
    if owner["processes"]:
        write_owner(owner)
    reply = {"ok": True}
    if SESSION.exists():
        try:
            reply = request({"command": "stop"}, timeout=45)
        except DriveError:
            pass  # The owned process tree still gets cleaned up after a failed host.
    # Let Flutter close its vault and complete test teardown first.
    deadline = time.monotonic() + 5
    while owner["processes"] and time.monotonic() < deadline:
        if not owned_processes(owner, processes()):
            break
        time.sleep(0.1)
    remaining = kill_owned(owner)
    if remaining:
        raise DriveError("session processes remain: " + ", ".join(str(r["ProcessId"]) for r in remaining))
    SESSION.unlink(missing_ok=True)
    OWNER.unlink(missing_ok=True)
    return reply


def quoted(value: object) -> str:
    return json.dumps(value, ensure_ascii=False)


def number(value: object) -> str:
    return f"{float(value):g}"


def format_reply(reply: dict, command: str) -> str:
    if not reply.get("ok"):
        return "error: " + " ".join(str(reply.get("error", "unknown host error")).splitlines())
    lines = ["ok"]
    if command == "look":
        lines.append(str(reply.get("path", "")))
        for entry in reply.get("entries", []):
            ref = str(entry["ref"]).strip("[]")
            line = (f'[{ref}] {entry["role"]} {quoted(entry.get("label", ""))} '
                    f'@{number(entry["x"])},{number(entry["y"])} '
                    f'{number(entry["width"])}x{number(entry["height"])}')
            states = entry.get("states", [])
            if isinstance(states, dict):
                states = [k for k, v in states.items() if v]
            if states:
                line += " " + ", ".join(states)
            if "value" in entry:
                line += " value=" + quoted(entry["value"])
            lines.append(line)
    elif command == "state":
        lines += ["title: " + quoted(reply.get("title")),
                  "routes: " + " > ".join(quoted(t) for t in reply.get("routes", [])),
                  "focused: " + quoted(reply.get("focused")),
                  f'pending: {reply.get("pendingDebounce", 0)} debounces, {reply.get("pendingBridge", 0)} bridge calls']
        if reply.get("vaultPath"):
            lines.append("vault: " + str(reply["vaultPath"]))
    elif reply.get("message"):
        lines.append(str(reply["message"]))
    return "\n".join(lines)


MSYS_NOTE = """\
Git Bash (MSYS) rewrites arguments that start with / into Windows paths, so
`type /head` arrives as `C:/Program Files/Git/head`. Use `type --slash head`
(or `key --slash`), or set MSYS_NO_PATHCONV=1 for the command:
  MSYS_NO_PATHCONV=1 python app/tool/drive.py type /head"""


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__, epilog=MSYS_NOTE,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = result.add_subparsers(dest="command", required=True)

    def form(p):
        g = p.add_mutually_exclusive_group()
        g.add_argument("--desktop", action="store_true")
        g.add_argument("--mobile", action="store_true")
        g.add_argument("--size", type=size)

    begin = sub.add_parser("start")
    form(begin)
    begin.add_argument("--dark", action="store_true")
    begin.add_argument("--vault", default="fresh")
    begin.add_argument("--now")
    for name in ("tap", "double", "long", "right", "hover"):
        p = sub.add_parser(name)
        p.add_argument("target")
        p.add_argument("--no-settle", action="store_true")
    p = sub.add_parser("look")
    p.add_argument("--label")
    p.add_argument("--no-settle", action="store_true")
    p = sub.add_parser("drag")
    p.add_argument("from", metavar="from")
    p.add_argument("to")
    p.add_argument("--no-settle", action="store_true")
    p = sub.add_parser("scroll")
    p.add_argument("target")
    p.add_argument("dy", type=float)
    p.add_argument("--no-settle", action="store_true")
    p = sub.add_parser(
        "type",
        help="type text into the focused field, one character at a time",
        description="Types TEXT into the focused field one character at a time, settling "
                    "briefly between characters, so pickers such as @ and [[ open as they "
                    r"would for a person. \n in TEXT presses Enter.",
        epilog=MSYS_NOTE,
        formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("text", nargs="?", default="")
    p.add_argument("--paste", action="store_true",
                   help="insert each line in one edit instead (the old behaviour)")
    p.add_argument("--slash", "--key-slash", dest="slash", action="store_true",
                   help="type a / first (safe from Git Bash path rewriting)")
    p.add_argument("--no-settle", action="store_true")
    p = sub.add_parser("key", help="press a key or combination, such as ctrl+n or enter",
                       epilog=MSYS_NOTE, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("combo", nargs="?")
    p.add_argument("--slash", "--key-slash", dest="slash", action="store_true",
                   help="press / (safe from Git Bash path rewriting)")
    p.add_argument("--no-settle", action="store_true")
    p = sub.add_parser("wait")
    p.add_argument("ms", type=int)
    p = sub.add_parser("theme")
    p.add_argument("theme", choices=("light", "dark"))
    p.add_argument("--no-settle", action="store_true")
    p = sub.add_parser("resize")
    form(p)
    p.add_argument("--no-settle", action="store_true")
    sub.add_parser("state")
    sub.add_parser("stop")
    return result


def main(argv: list[str] | None = None) -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    args = parser().parse_args(argv)
    try:
        if args.command == "wait" and args.ms < 0:
            raise DriveError("wait duration must be non-negative")
        if args.command == "start":
            start(args)
            return 0
        if args.command == "stop":
            reply = stop()
        else:
            payload = vars(args).copy()
            if "no_settle" in payload:
                payload["noSettle"] = payload.pop("no_settle")
            if args.command == "resize":
                payload = dict(command="resize", **dimensions(args), noSettle=args.no_settle)
            if args.command == "theme":
                payload["dark"] = payload.pop("theme") == "dark"
            if args.command == "type":
                payload["text"] = ("/" if payload.pop("slash") else "") + (args.text or "")
                if not payload["text"]:
                    raise DriveError("type needs text or --slash")
            if args.command == "key":
                slash = payload.pop("slash")
                if slash == bool(args.combo):
                    raise DriveError("key needs one combo, or --slash")
                if slash:
                    payload["combo"] = "/"
            reply = request(payload)
        print(format_reply(reply, args.command))
        return 0 if reply.get("ok") else 1
    except (DriveError, OSError, subprocess.SubprocessError) as exc:
        print("error: " + " ".join(str(exc).splitlines()))
        return 1
    except KeyboardInterrupt:
        print("error: interrupted")
        return 130


if __name__ == "__main__":
    sys.exit(main())
