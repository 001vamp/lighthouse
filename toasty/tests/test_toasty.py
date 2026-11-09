"""Toasty tests. Run with tests/run (a private D-Bus session, fake server, fake kitty)."""
import hashlib, json, os, shutil, signal, subprocess, sys, tempfile, time, unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, 'bin')

def wait_for(check, timeout=5.0):
    end = time.time() + timeout
    while time.time() < end:
        result = check()
        if result:
            return result
        time.sleep(0.05)
    return check()


class Env(unittest.TestCase):
    """Each test gets its own runtime dir, state dir, PATH with a fake kitty, and a fake server."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix='toasty-test-')
        self.runtime = os.path.join(self.tmp, 'run')
        os.makedirs(self.runtime, mode=0o700)
        self.fakebin = os.path.join(self.tmp, 'fakebin')
        os.makedirs(self.fakebin)
        self.env = dict(os.environ, XDG_RUNTIME_DIR=self.runtime, XDG_STATE_HOME=os.path.join(self.tmp, 'state'),
                        XDG_DATA_HOME=os.path.join(self.tmp, 'data'), HOME=self.tmp,
                        PATH=self.fakebin + os.pathsep + BIN + os.pathsep + os.environ['PATH'])
        # Nothing from the desktop this runs on: no kitty, no agent app, and a fake niri
        # that reports no focused window.
        for key in list(self.env):
            if key.startswith(('KITTY_', 'CLAUDE', 'CODEX_', 'NIRI_')):
                self.env.pop(key)
        niri = os.path.join(self.fakebin, 'niri')
        with open(niri, 'w') as f:
            f.write('#!/bin/sh\ncase "$*" in *focused-window*) echo null;; *windows*) echo "[]";; esac\n')
        os.chmod(niri, 0o755)
        self.server_log = os.path.join(self.tmp, 'server.jsonl')
        self.server = subprocess.Popen([sys.executable, os.path.join(ROOT, 'tests', 'fake_server.py'), self.server_log])
        self.assertTrue(wait_for(lambda: any(e['method'] == 'ready' for e in self.calls())), 'fake server up')
        self.procs = []

    def tearDown(self):
        for p in self.procs:
            if p.poll() is None:
                p.kill()
        self.server.terminate()
        self.server.wait()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def calls(self, method=None):
        try:
            with open(self.server_log) as f:
                entries = [json.loads(line) for line in f]
        except FileNotFoundError:
            return []
        return [e for e in entries if method is None or e['method'] == method]

    def notify(self, *args, env=None):
        p = subprocess.Popen([os.path.join(BIN, 'toasty-notify'), *args], env=env or self.env,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.procs.append(p)
        return p

    def user(self, member, nid, arg=''):
        subprocess.run(['gdbus', 'call', '--session', '--dest', 'org.freedesktop.Notifications',
                        '--object-path', '/org/freedesktop/Notifications', '--method',
                        'org.freedesktop.Notifications.TestEmit', member, str(nid), arg],
                       check=True, capture_output=True)

    def markers(self):
        d = os.path.join(self.runtime, 'toasty', 'markers')
        return sorted(os.listdir(d)) if os.path.isdir(d) else []

    def fake_kitty(self, foreground):
        """A kitty that reports `foreground` as the window's foreground cmdline and logs what's typed."""
        typed = os.path.join(self.tmp, 'typed.txt')
        script = f'''#!/usr/bin/env python3
import json, sys
args = sys.argv[1:]
if 'ls' in args:
    print(json.dumps([{{"tabs": [{{"is_focused": True, "windows": [{{"id": 7, "title": "agent", "is_focused": False,
        "foreground_processes": [{{"cmdline": {json.dumps(foreground)}}}]}}]}}]}}]))
elif 'send-text' in args:
    open({typed!r}, 'a').write('TEXT:' + sys.stdin.read() + '\\n')
elif 'send-key' in args:
    open({typed!r}, 'a').write('KEY:' + args[-1] + '\\n')
'''
        path = os.path.join(self.fakebin, 'kitty')
        with open(path, 'w') as f:
            f.write(script)
        os.chmod(path, 0o755)
        env = dict(self.env, KITTY_WINDOW_ID='7', KITTY_LISTEN_ON='unix:/nonexistent', KITTY_PID='1')
        return env, typed


class Wire(Env):
    def test_identity_hints_and_actions(self):
        p = self.notify('--agent', 'Claude Code', '--provider', 'Anthropic', '--kind', 'needs',
                        '--context', 'api', '--code', 'git push', '--action', 'allow=Allow',
                        '--action', 'deny=Deny', 'Run this command?', 'caption')
        n = wait_for(lambda: self.calls('Notify'))[0]
        self.assertEqual(n['app'], 'Toasty|Claude Code|Anthropic||')
        self.assertEqual(n['hints']['x-toasty-kind'], 'needs')
        self.assertEqual(n['hints']['x-toasty-context'], 'api')
        self.assertEqual(n['hints']['x-toasty-code'], 'git push')
        self.assertEqual(n['hints']['urgency'], 2)
        self.assertTrue(n['hints']['resident'])
        self.assertEqual(n['actions'], ['allow', 'Allow', 'deny', 'Deny'])
        self.assertFalse(any(k.startswith('x-pond-') for k in n['hints']))
        p.kill()

    def test_button_answer_reaches_the_caller(self):
        p = self.notify('--agent', 'Codex', '--kind', 'needs', '--action', 'allow=Allow', 'Run?')
        nid = wait_for(lambda: self.calls('Notify'))[0]['id']
        self.assertTrue(wait_for(lambda: f'{nid}.alive' in self.markers()), 'alive marker while listening')
        self.user('ActionInvoked', nid, 'allow')
        self.assertEqual(p.stdout.readline().strip(), 'action:allow')
        self.user('NotificationClosed', nid)
        p.wait(5)
        self.assertEqual(self.markers(), [], 'markers cleared on exit')

    def test_sigterm_withdraws_the_toast(self):
        p = self.notify('--agent', 'Codex', '--kind', 'needs', '--action', 'allow=Allow', 'Run?')
        nid = wait_for(lambda: self.calls('Notify'))[0]['id']
        wait_for(lambda: f'{nid}.alive' in self.markers())
        p.send_signal(signal.SIGTERM)
        p.wait(5)
        self.assertEqual([c['id'] for c in self.calls('CloseNotification')], [nid])

    def test_identity_fields_cannot_forge_the_separator(self):
        p = self.notify('--agent', 'Evil|Claude Code', 'x')
        p.wait(5)
        self.assertNotEqual(p.returncode, 0)
        self.assertEqual(self.calls('Notify'), [])

    def test_stale_markers_are_pruned(self):
        d = os.path.join(self.runtime, 'toasty', 'markers')
        os.makedirs(d, mode=0o700)
        dead = subprocess.Popen(['true']); dead.wait()
        with open(os.path.join(d, '999.alive'), 'w') as f:
            f.write(str(dead.pid))
        p = self.notify('--agent', 'Codex', '--action', 'a=A', 'x')
        wait_for(lambda: self.calls('Notify'))
        self.assertTrue(wait_for(lambda: '999.alive' not in self.markers()))
        p.kill()


class Replies(Env):
    def test_detached_without_kitty_offers_no_reply(self):
        p = self.notify('--agent', 'Codex', '--reply', '--detach', 'x')
        p.wait(5)
        n = wait_for(lambda: self.calls('Notify'))[0]
        self.assertNotIn('inline-reply', n['actions'])

    def test_foreground_reply_is_handed_over_without_a_delivered_claim(self):
        p = self.notify('--agent', 'Codex', '--reply', 'x')
        nid = wait_for(lambda: self.calls('Notify'))[0]['id']
        wait_for(lambda: f'{nid}.alive' in self.markers())
        self.user('NotificationReplied', nid, 'hello')
        self.assertEqual(p.stdout.readline().strip(), 'reply:hello')
        time.sleep(0.2)
        self.assertNotIn(f'{nid}.delivered', self.markers())

    def test_reply_typed_only_into_the_agent(self):
        env, typed = self.fake_kitty(['claude', '--resume'])
        p = self.notify('--agent', 'Claude Code', '--kitty', '--reply', '--detach', 'x', env=env)
        p.wait(5)
        nid = wait_for(lambda: self.calls('Notify'))[0]['id']
        wait_for(lambda: f'{nid}.alive' in self.markers())
        self.user('NotificationReplied', nid, 'continue')
        self.assertTrue(wait_for(lambda: f'{nid}.delivered' in self.markers()), 'delivered after typing')
        self.assertEqual(open(typed).read(), 'TEXT:continue\nKEY:enter\n')

    def test_reply_refused_when_a_shell_is_in_front(self):
        env, typed = self.fake_kitty(['bash', '-c', 'claude'])
        p = self.notify('--agent', 'Claude Code', '--kitty', '--reply', '--detach', 'x', env=env)
        p.wait(5)
        nid = wait_for(lambda: self.calls('Notify'))[0]['id']
        wait_for(lambda: f'{nid}.alive' in self.markers())
        self.user('NotificationReplied', nid, 'rm -rf ~')
        time.sleep(1.0)
        self.assertFalse(os.path.exists(typed), 'nothing typed into a shell')
        self.assertNotIn(f'{nid}.delivered', self.markers())

    def test_unknown_agent_never_gets_typed_into(self):
        env, typed = self.fake_kitty(['mystery'])
        p = self.notify('--agent', 'Mystery Bot', '--kitty', '--reply', '--detach', 'x', env=env)
        p.wait(5)
        nid = wait_for(lambda: self.calls('Notify'))[0]['id']
        wait_for(lambda: f'{nid}.alive' in self.markers())
        self.user('NotificationReplied', nid, 'hi')
        time.sleep(1.0)
        self.assertFalse(os.path.exists(typed))


class Hooks(Env):
    def run_hook(self, name, payload, env=None):
        return subprocess.run([os.path.join(BIN, name)], input=json.dumps(payload), text=True,
                              env=env or self.env, capture_output=True, timeout=15)

    def sessions(self):
        try:
            return json.load(open(os.path.join(self.runtime, 'toasty', 'sessions.json')))
        except FileNotFoundError:
            return {}

    def test_codex_status_lifecycle(self):
        base = {'session_id': 't1', 'cwd': '/work/api'}
        for event, state in (('UserPromptSubmit', 'working'), ('PermissionRequest', 'waiting'),
                             ('PostToolUse', 'working'), ('Stop', 'done')):
            self.run_hook('toasty-codex-status', {**base, 'hook_event_name': event})
            self.assertEqual(self.sessions()['t1']['state'], state, event)
        self.assertEqual(self.sessions()['t1']['open'], 'codex://threads/t1')
        self.run_hook('toasty-codex-status', {**base, 'hook_event_name': 'PostToolUse'})
        self.assertEqual(self.sessions()['t1']['state'], 'done', 'a tool finishing never undoes Done')
        self.run_hook('toasty-codex-status', {**base, 'hook_event_name': 'SessionEnd'})
        self.assertNotIn('t1', self.sessions())

    def test_codex_hidden_turns_are_ignored(self):
        self.run_hook('toasty-codex-status', {'session_id': 'h', 'cwd': '/w', 'transcript_path': None,
                                              'hook_event_name': 'UserPromptSubmit'})
        self.assertNotIn('h', self.sessions(), 'no transcript: not a thread you can open')
        notify = os.path.join(BIN, 'toasty-codex-notify')
        done = lambda thread: json.dumps({'type': 'agent-turn-complete', 'thread-id': thread, 'cwd': '/w',
                                          'last-assistant-message': 'ok'})
        subprocess.run([notify, done('hidden')], env=self.env, timeout=15)
        time.sleep(0.5)
        self.assertEqual(self.calls('Notify'), [], 'no rollout: no toast')
        self.assertNotIn('hidden', self.sessions())
        day = os.path.join(self.tmp, '.codex', 'sessions', '2026', '11', '09')
        os.makedirs(day)
        open(os.path.join(day, 'rollout-2026-11-09T09-44-20-saved.jsonl'), 'w').close()
        subprocess.run([notify, done('saved')], env=self.env, timeout=15)
        self.assertTrue(wait_for(lambda: self.calls('Notify')), 'a saved thread still toasts')
        self.assertEqual(self.sessions()['saved']['state'], 'done')

    def test_codex_in_a_terminal_is_not_the_app(self):
        codex = os.path.join(self.fakebin, 'codex')
        with open(codex, 'w') as f:
            f.write('#!/bin/sh\n"$@"\ntrue\n')
        os.chmod(codex, 0o755)
        payload = json.dumps({'session_id': 't', 'cwd': '/w', 'hook_event_name': 'UserPromptSubmit'})
        subprocess.run([codex, os.path.join(BIN, 'toasty-codex-status')], input=payload, text=True, env=self.env)
        self.assertEqual(self.sessions()['t']['app'], '')
        self.assertNotIn('open', self.sessions()['t'])

    def test_app_focus_only_silences_the_toast_for_a_lone_session(self):
        niri = os.path.join(self.fakebin, 'niri')
        with open(niri, 'w') as f:
            f.write('#!/bin/sh\ncase "$*" in *focused-window*) echo \'{"app_id": "com.anthropic.Claude"}\';; esac\n')
        env = dict(self.env, CLAUDE_CODE_DESKTOP_APP_VERSION='1')
        stop = {'hook_event_name': 'Stop', 'session_id': 'one', 'cwd': '/w/a'}
        self.run_hook('toasty-claude-hook', stop, env)
        time.sleep(0.5)
        self.assertEqual(self.calls('Notify'), [], 'looking at the only session: no toast')
        self.run_hook('toasty-claude-hook', {**stop, 'session_id': 'two', 'hook_event_name': 'UserPromptSubmit'}, env)
        self.run_hook('toasty-claude-hook', stop, env)
        self.assertTrue(wait_for(lambda: self.calls('Notify')), 'another session is open: toast')

    def test_codex_status_skips_kitty(self):
        env = dict(self.env, KITTY_WINDOW_ID='3')
        self.run_hook('toasty-codex-status', {'session_id': 'k', 'hook_event_name': 'UserPromptSubmit'}, env)
        self.assertNotIn('k', self.sessions())

    def test_claude_app_session_states(self):
        env = dict(self.env, CLAUDE_CODE_DESKTOP_APP_VERSION='1')
        base = {'session_id': 'c1', 'cwd': '/work/lo'}
        self.run_hook('toasty-claude-hook', {**base, 'hook_event_name': 'UserPromptSubmit'}, env)
        self.assertEqual(self.sessions()['c1'], {**self.sessions()['c1'], 'agent': 'Claude Code', 'state': 'working',
                                                 'app': 'com.anthropic.Claude', 'project': 'lo'})
        self.run_hook('toasty-claude-hook', {**base, 'hook_event_name': 'SessionEnd'}, env)
        self.assertNotIn('c1', self.sessions())

    def test_codex_permission_allow_and_deny(self):
        payload = {'hook_event_name': 'PermissionRequest', 'session_id': 's', 'cwd': '/w', 'tool_name': 'Bash',
                   'tool_input': {'command': 'npm test'}}
        for button, behavior in (('allow', 'allow'), ('deny', 'deny')):
            before = len(self.calls('Notify'))
            p = subprocess.Popen([os.path.join(BIN, 'toasty-codex-permission')], stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, text=True, env=self.env)
            p.stdin.write(json.dumps(payload)); p.stdin.close()
            n = wait_for(lambda: len(self.calls('Notify')) > before and self.calls('Notify')[-1])
            self.assertEqual(n['hints']['x-toasty-code'], 'npm test')
            self.assertNotIn('always', n['actions'], 'Codex can\'t remember rules, so no Always allow')
            wait_for(lambda: f"{n['id']}.alive" in self.markers())
            self.user('ActionInvoked', n['id'], button)
            out = json.loads(p.stdout.read())
            p.wait(5)
            self.assertEqual(out['hookSpecificOutput']['decision']['behavior'], behavior)
            self.user('NotificationClosed', n['id'])

    def test_claude_permission_always_only_for_allow_rules(self):
        payload = {'hook_event_name': 'PermissionRequest', 'session_id': 's', 'cwd': '/w', 'tool_name': 'Bash',
                   'tool_input': {'command': 'git init'},
                   'permission_suggestions': [
                       {'type': 'setMode', 'mode': 'acceptEdits', 'destination': 'session'},
                       {'type': 'addRules', 'behavior': 'allow', 'destination': 'localSettings',
                        'rules': [{'toolName': 'Bash', 'ruleContent': 'git init *'}]}]}
        p = subprocess.Popen([os.path.join(BIN, 'toasty-claude-permission')], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, text=True, env=self.env)
        p.stdin.write(json.dumps(payload)); p.stdin.close()
        n = wait_for(lambda: self.calls('Notify'))[0]
        self.assertIn('always', n['actions'])
        self.assertIn('git init *', n['body'])
        wait_for(lambda: f"{n['id']}.alive" in self.markers())
        self.user('ActionInvoked', n['id'], 'always')
        decision = json.loads(p.stdout.read())['hookSpecificOutput']['decision']
        self.assertEqual(decision['behavior'], 'allow')
        self.assertEqual([s['type'] for s in decision['updatedPermissions']], ['addRules'],
                         'never passes on a mode switch')

    def test_permission_gives_no_decision_when_the_toast_closes(self):
        payload = {'hook_event_name': 'PermissionRequest', 'session_id': 's', 'cwd': '/w', 'tool_name': 'Bash',
                   'tool_input': {'command': 'ls'}}
        p = subprocess.Popen([os.path.join(BIN, 'toasty-claude-permission')], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, text=True, env=self.env)
        p.stdin.write(json.dumps(payload)); p.stdin.close()
        n = wait_for(lambda: self.calls('Notify'))[0]
        wait_for(lambda: f"{n['id']}.alive" in self.markers())
        self.user('NotificationClosed', n['id'])
        self.assertEqual(p.stdout.read().strip(), '', 'the agent\'s own prompt decides')


class Install(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix='toasty-home-')
        os.makedirs(os.path.join(self.home, '.claude'))
        os.makedirs(os.path.join(self.home, '.codex'))
        self.settings = os.path.join(self.home, '.claude', 'settings.json')
        json.dump({'theme': 'dark', 'hooks': {
            'Stop': [{'hooks': [{'type': 'command', 'command': '/usr/bin/my-own-hook'},
                                {'type': 'command', 'command': '/home/x/.local/bin/pond-claude-hook'}]}]}},
                  open(self.settings, 'w'))
        self.codex_hooks = os.path.join(self.home, '.codex', 'hooks.json')
        json.dump({'hooks': {'Stop': [{'hooks': [{'type': 'command', 'command': '/opt/other'}]}]}},
                  open(self.codex_hooks, 'w'))
        self.config = os.path.join(self.home, '.codex', 'config.toml')
        open(self.config, 'w').write('model = "x"\n\n[projects."/a"]\ntrust_level = "trusted"\n')
        self.env = dict(os.environ, HOME=self.home, XDG_DATA_HOME=os.path.join(self.home, 'data'),
                        TOASTY_BIN=os.path.join(self.home, 'bin'), XDG_CONFIG_HOME=os.path.join(self.home, 'config'),
                        TOASTY_POND_PACKAGED=os.path.join(self.home, 'no-pond'))
        self.overlay = os.path.join(self.home, 'config', 'quickshell', 'pond-shell')

    def fake_pond(self):
        """A packaged Pond that Toasty's patch doesn't fit, like a future Pond version would be."""
        pond = self.env['TOASTY_POND_PACKAGED']
        os.makedirs(os.path.join(pond, 'notifications'))
        for name in ('Sidebar.qml', 'notifications/State.qml', 'notifications/ToastWindow.qml', 'notifications/SidebarCard.qml'):
            open(os.path.join(pond, name), 'w').write('Item {}\n')

    def tearDown(self):
        shutil.rmtree(self.home, ignore_errors=True)

    def toasty(self, *args):
        r = subprocess.run([os.path.join(BIN, 'toasty'), *args], env=self.env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout

    def commands(self, file):
        return [h['command'] for gs in json.load(open(file)).get('hooks', {}).values() for g in gs for h in g['hooks']]

    def test_install_is_additive_idempotent_and_reversible(self):
        self.toasty('install')
        cmds = self.commands(self.settings)
        self.assertIn('/usr/bin/my-own-hook', cmds)
        self.assertIn('/home/x/.local/bin/pond-claude-hook', cmds, 'legacy kept without --migrate')
        self.assertTrue(any(c.endswith('/toasty-claude-permission') for c in cmds))
        self.assertEqual(json.load(open(self.settings))['theme'], 'dark')
        self.assertIn('/opt/other', self.commands(self.codex_hooks))
        config = open(self.config).read()
        self.assertTrue(config.index('notify = ') < config.index('[projects'), 'notify stays top-level')
        snapshot = (open(self.settings).read(), open(self.codex_hooks).read(), config)
        self.assertIn('unchanged', self.toasty('install'))
        self.assertEqual(snapshot, (open(self.settings).read(), open(self.codex_hooks).read(), open(self.config).read()))

        self.toasty('install', '--migrate')
        self.assertNotIn('/home/x/.local/bin/pond-claude-hook', self.commands(self.settings))

        self.toasty('uninstall')
        self.assertEqual(self.commands(self.settings), ['/usr/bin/my-own-hook'])
        self.assertEqual(self.commands(self.codex_hooks), ['/opt/other'])
        self.assertNotIn('notify', open(self.config).read())
        self.assertIn('[projects."/a"]', open(self.config).read())
        self.assertFalse(os.path.exists(os.path.join(self.home, 'bin', 'toasty-notify')))

    def test_registry_updates_unless_you_edited_it(self):
        registry = os.path.join(self.home, 'data', 'toasty', 'agents.json')
        self.toasty('install')
        shipped = open(registry).read()
        open(registry, 'w').write('{"mine": {}}')
        self.toasty('install')
        self.assertEqual(open(registry).read(), '{"mine": {}}', 'your edit is kept')
        self.assertEqual(open(registry + '.new').read(), shipped, 'the new version is beside it')
        with open(registry + '.installed', 'w') as f:
            f.write(hashlib.sha256(b'{"mine": {}}').hexdigest())
        self.toasty('install')
        self.assertEqual(open(registry).read(), shipped, 'not edited since Toasty wrote it: updated')
        self.assertFalse(os.path.exists(registry + '.new'))

    def test_never_replaces_someone_elses_notify(self):
        open(self.config, 'w').write('notify = ["/usr/bin/other-notifier"]\n')
        out = self.toasty('install')
        self.assertIn('left alone', out)
        self.assertIn('other-notifier', open(self.config).read())

    def test_shell_left_alone_when_the_patch_does_not_fit(self):
        self.fake_pond()
        out = self.toasty('install')
        self.assertIn("don't fit this Pond", out)
        self.assertFalse(os.path.exists(self.overlay), 'the packaged Pond keeps running as it is')
        self.assertFalse(os.path.exists(self.overlay + '.toasty-tmp'))

    def test_never_touches_a_pond_config_that_is_not_ours(self):
        self.fake_pond()
        os.makedirs(self.overlay)
        open(os.path.join(self.overlay, 'shell.qml'), 'w').write('// mine\n')
        self.assertIn('left alone', self.toasty('install'))
        self.toasty('uninstall')
        self.assertEqual(open(os.path.join(self.overlay, 'shell.qml')).read(), '// mine\n')


if __name__ == '__main__':
    unittest.main()
