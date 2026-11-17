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


def _t3_link(link):
    """T3's web link for a thread → the t3code:// link its desktop app opens."""
    prefix = 'https://app.t3.codes/threads/'
    return 't3code://threads/' + link[len(prefix):] if isinstance(link, str) and link.startswith(prefix) else ''


def t3_thread(session, lookup):
    """The T3 Code thread behind a session T3 started: {"thread", "open", "job"?}, where "job"
    ({"id", "title", "open"}) is the orchestrator thread that spawned it. Looked up once per
    session with T3's own CLI and kept in sessions.json; {} when the lookup failed. None for
    sessions T3 didn't start, or when this event shouldn't wait on a lookup."""
    thread = os.environ.get('T3CODE_THREAD_ID')
    if not thread:
        return None
    try:
        with open(os.path.join(RUNTIME, 'sessions.json')) as f:
            known = json.load(f).get(session, {}).get('t3')
    except (OSError, ValueError, AttributeError):
        known = None
    if known is not None or not lookup:
        return known
    import subprocess
    cli = os.environ.get('T3CODE_CLI_PATH') or 't3'

    def show(tid):
        try:
            out = subprocess.run([cli, 'thread', 'show', tid, '--json', '--messages', '0'],
                                 capture_output=True, text=True, timeout=1.5, stdin=subprocess.DEVNULL).stdout
            found = json.loads(out).get('thread')
            return found if isinstance(found, dict) else None
        except (OSError, ValueError, AttributeError, subprocess.SubprocessError):
            return None

    me = show(thread)
    if not me:
        return {}
    info = {'thread': thread, 'open': _t3_link(me.get('link'))}
    parent = me.get('parentThreadId')
    if isinstance(parent, str) and parent:
        job = show(parent) or {}
        info['job'] = {'id': parent, 'title': str(job.get('title') or '')[:200], 'open': _t3_link(job.get('link'))}
    return info


def with_t3(entry, info):
    """Point a session's dock row at its T3 thread instead of the app it runs under."""
    if info:
        entry['t3'] = info
        if info.get('open'):
            entry.update(app='', open=info['open'])
    elif info == {}:
        entry['t3'] = {}
    return entry
