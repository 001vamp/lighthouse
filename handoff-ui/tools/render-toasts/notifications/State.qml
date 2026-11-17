pragma Singleton
import QtQuick
QtObject {
  property var queue: []
  property var stamps: ({})
  property var allowedAt: ({})
  property bool reducedMotion: true
  property bool away: false
  property string markerDir: "/nonexistent"
  property var aliveCheckCommand: ["true"]
  function agentInfo(n) { return { name: n.agentName || "Claude" }; }
  function agent(key) { return ({ claude: { mascot: "claudey.png" }, codex: { mascot: "codex.png" }, grokbot: { mascot: "ghost-grok.png" } })[key]; }
  function message(n) { return n.body ? String(n.body) : String(n.summary || ""); }
  function relativeTimeLabel(t) { const m = Math.floor((Date.now() - t) / 60000); return m < 1 ? "Now" : m + "m"; }
  function openLine(n, l) {}
  function agentForTitle(t) { return null; }
}
