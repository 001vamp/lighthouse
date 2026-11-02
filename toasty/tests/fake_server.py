#!/usr/bin/env python3
"""A stand-in notification server for tests: records every call as a JSON line in $1, and
has a TestEmit method so a test can play the user (click a button, reply, close)."""
import json, sys
import gi
gi.require_version('Gio', '2.0')
from gi.repository import Gio, GLib

LOG = sys.argv[1]
XML = '''<node><interface name="org.freedesktop.Notifications">
 <method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/>
  <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/>
  <arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
 <method name="CloseNotification"><arg type="u" direction="in"/></method>
 <method name="GetCapabilities"><arg type="as" direction="out"/></method>
 <method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/>
  <arg type="s" direction="out"/><arg type="s" direction="out"/></method>
 <method name="TestEmit"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/></method>
 <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
 <signal name="NotificationReplied"><arg type="u"/><arg type="s"/></signal>
 <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
</interface></node>'''

next_id = [0]

def record(entry):
    with open(LOG, 'a') as f:
        f.write(json.dumps(entry) + '\n')

def emit(conn, member, args):
    conn.emit_signal(None, '/org/freedesktop/Notifications', 'org.freedesktop.Notifications', member, args)

def on_call(conn, sender, path, iface, method, params, invocation):
    if method == 'Notify':
        app, _replaces, icon, summary, body, actions, hints, timeout = params.unpack()
        next_id[0] += 1
        record({'method': 'Notify', 'id': next_id[0], 'app': app, 'summary': summary, 'body': body,
                'actions': actions, 'hints': {k: v for k, v in hints.items()}})
        invocation.return_value(GLib.Variant('(u)', (next_id[0],)))
    elif method == 'CloseNotification':
        (nid,) = params.unpack()
        record({'method': 'CloseNotification', 'id': nid})
        invocation.return_value(None)
        emit(conn, 'NotificationClosed', GLib.Variant('(uu)', (nid, 3)))
    elif method == 'GetCapabilities':
        invocation.return_value(GLib.Variant('(as)', (['actions', 'body', 'inline-reply'],)))
    elif method == 'GetServerInformation':
        invocation.return_value(GLib.Variant('(ssss)', ('fake', 'toasty-tests', '1', '1.2')))
    elif method == 'TestEmit':
        member, nid, arg = params.unpack()
        invocation.return_value(None)
        if member == 'NotificationClosed':
            emit(conn, member, GLib.Variant('(uu)', (nid, 2)))
        else:
            emit(conn, member, GLib.Variant('(us)', (nid, arg)))

def on_bus(conn, _name):
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    conn.register_object('/org/freedesktop/Notifications', node.interfaces[0], on_call, None, None)

def on_name(_conn, _name):
    record({'method': 'ready'})

Gio.bus_own_name(Gio.BusType.SESSION, 'org.freedesktop.Notifications', Gio.BusNameOwnerFlags.NONE,
                 on_bus, on_name, None)
GLib.MainLoop().run()
