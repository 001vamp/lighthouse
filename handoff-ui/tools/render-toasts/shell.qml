import QtQuick
import Quickshell
import "notifications" as Notifications
ShellRoot {
  FloatingWindow {
    id: win
    implicitWidth: 440; implicitHeight: 1400
    color: "#1F1D23"
    Column {
      id: col
      x: 10; y: 10; spacing: 4
      Repeater {
        model: [
          { agentName: "Claude", kind: "needs", context: "pond-interface", restored: true, body: "Which branch should I base this on?", windowId: 3, timestamp: Date.now() - 2*3600000 },
          { agentName: "Claude", kind: "done", context: "lighthouse", body: "Finished the dock lifecycle fix. **25 tests** passed and CI is green, so this is ready for review by Sol. I also cleaned up the old worktrees and deleted the merged PR branches, and left the backup tarball where it was.", hasInlineReply: true, windowId: 3, timestamp: Date.now() },
          { agentName: "Claude", kind: "done", context: "lighthouse", body: "Done. All 25 tests pass.", windowId: 3, timestamp: Date.now() - 60000, sentDemo: "run the tests again" },
          { agentName: "Codex", kind: "done", context: "in", body: "Finished the refactor.\n\n- Moved the hook into toastylib\n- Kept `--details` working\n- Updated PROTOCOL.md\n\nEverything else is unchanged. Tests pass locally and I left a note in the handoff for the next session about the kitty remote-control setup.", windowId: 5, timestamp: Date.now(), expandDemo: true },
          { agentName: "Codex", kind: "needs", reason: "approval", context: "pond-interface", summary: "Wants to run a command", code: "rm -rf build && qmake6 && make -j8", details: "{\n  \"command\": \"rm -rf build && qmake6 && make -j8\"\n}", openUrl: "codex://x", timestamp: Date.now() - 120000 },
          { agentName: "Claude", kind: "paused", reason: "Usage limit reached", context: "usage-demo", body: "You've hit your usage limit. It resets at 4:10pm (America/Chicago). Run /upgrade to raise it.", windowId: 3, timestamp: Date.now() },
          { agentName: "Claude", kind: "paused", reason: "Usage limit reached", context: "usage-demo", body: "You've hit your usage limit. It resets at 4:10pm (America/Chicago). Run /upgrade to raise it.", windowId: 3, timestamp: Date.now(), expandDemo: true },
          { agentName: "Grok Bot", kind: "failed", context: "hooks", body: "Rate limit reached", windowId: 4, timestamp: Date.now() - 300000 },
          { agentName: "Claude", kind: "needs", context: "pond-interface", body: "Which branch should I base this on, main or docs/toasty-design?", hasInlineReply: true, windowId: 3, timestamp: Date.now() - 720000 }
        ]
        delegate: Notifications.AgentToast { required property var modelData; notification: modelData; width: implicitWidth; height: implicitHeight
          Timer { interval: 300; running: true; onTriggered: { if (parent.modelData.expandDemo) parent.expanded = true; if (parent.modelData.sentDemo) { parent.sentText = parent.modelData.sentDemo; parent.delivery = "sent"; } } } }
      }
    }
    Timer {
      interval: 3000; running: true
      onTriggered: col.grabToImage(r => { r.saveToFile(Quickshell.env("OUT")); Qt.quit(); })
    }
  }
}
