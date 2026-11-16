"""Helpers shared by Toasty's hooks. `toasty install` puts this file next to them."""
import json, os, time

RUNTIME = os.path.join(os.environ.get('XDG_RUNTIME_DIR') or f'/tmp/toasty-{os.getuid()}', 'toasty')


def other_app_sessions(session, app):
    """Is another session of this desktop app active in the last hour? Then the app having focus
    doesn't mean you're looking at this session, so it shouldn't silence the toast."""
    try:
        with open(os.path.join(RUNTIME, 'sessions.json')) as f:
            sessions = json.load(f)
    except (OSError, ValueError):
        return False
    return any(sid != session and s.get('app') == app and time.time() - s.get('time', 0) < 3600
               for sid, s in sessions.items())


def in_desktop_app():
    """False only when a terminal Codex started us: the ChatGPT app's Codex runs as app-server."""
    pid = os.getppid()
    for _ in range(8):
        try:
            with open(f'/proc/{pid}/cmdline', 'rb') as f:
                argv = f.read().decode('utf-8', 'replace').split('\0')
            with open(f'/proc/{pid}/stat') as f:
                parent = int(f.read().rpartition(')')[2].split()[1])
        except (OSError, ValueError, IndexError):
            return True
        if 'codex' in (os.path.basename(a) for a in argv[:2]):
            return 'app-server' in argv or 'exec-server' in argv
        if parent <= 1:
            return True
        pid = parent
    return True
