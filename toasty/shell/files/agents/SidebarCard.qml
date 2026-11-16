import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Io
import ".." as Shell
import "../notifications" as Notifications

// Agents parked in the sidebar: one row per agent, with the library's status pill.
// Kitty sessions: state comes from the terminal title (see NiriMsg._isWorking/_needsAttention),
// so it works for every Claude Code / Codex session, including ones started before Pond.
// Which agent a window is comes from the title patterns in Toasty's agent registry.
// Desktop app sessions: state comes from hooks, via $XDG_RUNTIME_DIR/toasty/sessions.json.
//
// Done: always shows for `doneGrace` with the library's pop (pill pops, turns blue, green
// check springs in), even if you were looking. After that it stays Done until you've seen
// it; then a terminal row settles to Idle. App rows clear only when individually opened or dismissed.
Shell.Card {
  id: root
  color: "#29272D"
  property real collapseProgress: 0
  readonly property real widthProgress: Shell.Theme.collapseWidth(collapseProgress)
  readonly property real heightProgress: Shell.Theme.collapseHeight(collapseProgress)
  readonly property int rowHeight: Math.round(Shell.Theme.lerp(58, 40, heightProgress))
  readonly property int doneGrace: 6000
  readonly property int leaveDuration: 400

  // Titles can't say a turn died, so hooks report that here, keyed by niri window
  // ("win:<id>", or the kitty pid when the window couldn't be told apart):
  // {"win:66": {"error": "Rate limit reached", "time": …}}. The next good turn clears it.
  readonly property string failuresPath: Quickshell.env("XDG_RUNTIME_DIR") + "/toasty/failures.json"
  property var failed: ({})
  FileView {
    id: failures
    path: root.failuresPath
    watchChanges: true
    printErrors: false
    onFileChanged: reload()
    onLoaded: { try { root.failed = JSON.parse(text() || "{}"); } catch (e) { root.failed = {}; } }
  }
  // Sessions with no terminal title to read (the Claude and Codex desktop apps), reported by
  // their hooks: {"<session>": {"agent", "project", "state", "time", "app", "open"?}}.
  // "open" is a link back to the exact thread, e.g. codex://threads/<id>.
  readonly property string sessionsPath: Quickshell.env("XDG_RUNTIME_DIR") + "/toasty/sessions.json"
  property var sessions: ({})
  FileView {
    id: sessionFile
    path: root.sessionsPath
    watchChanges: true
    printErrors: false
    onFileChanged: reload()
    onLoaded: { try { root.sessions = JSON.parse(text() || "{}"); } catch (e) { root.sessions = {}; } }
  }
  // The files have to exist to be watched.
  Process {
    running: true
    command: ["sh", "-c", 'for f; do mkdir -p -m 700 "${f%/*}"; [ -e "$f" ] || echo "{}" > "$f"; done', "sh",
              root.failuresPath, root.sessionsPath]
    onExited: { failures.reload(); sessionFile.reload(); }
  }

  // Terminal rows: when a title stops showing work, remember when (window id → ms).
  property var lastRaw: ({})
  // The agent each window last showed itself to be, so an idle title keeps its name and mascot.
  property var knownAgent: ({})
  property var doneAt: ({})
  property var settlingAt: ({})
  // Re-evaluate hook freshness even when no new hook or window event arrives.
  readonly property int statusFreshness: 10 * 60 * 1000
  Timer { interval: 30000; repeat: true; running: true; onTriggered: root.refresh() }
  // Ticks while something is mid-grace or fading out.
  property bool ticking: false
  Timer { interval: 250; repeat: true; running: root.ticking; onTriggered: root.refresh() }

  readonly property var inputs: [Shell.NiriMsg._windows, failed, sessions, agentPids]
  onInputsChanged: Qt.callLater(refresh)
  Component.onCompleted: refresh()

  // A kitty window is an agent only while an agent's program runs inside it, or while its title
  // shows an agent's marks. A plain shell never gets a row, whatever its title says.
  // kitty pid → the registry entry whose `command` runs under it.
  property var agentPids: ({})
  readonly property bool hasKitty: Object.values(Shell.NiriMsg._windows).some(w => w.appId === "kitty")
  Process {
    id: processes
    command: ["ps", "-eo", "pid=,ppid=,comm="]
    stdout: StdioCollector { onStreamFinished: root.readProcesses(text) }
  }
  Timer {
    interval: 2000; repeat: true; triggeredOnStart: true
    running: root.hasKitty
    onTriggered: processes.running = true
    onRunningChanged: if (!running) root.agentPids = {}
  }
  function readProcesses(text) {
    const children = {}, comm = {};
    for (const line of text.split("\n")) {
      const m = line.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/);
      if (!m) continue;
      comm[m[1]] = m[3];
      (children[m[2]] = children[m[2]] || []).push(m[1]);
    }
    const byCommand = {};
    for (const id in Notifications.State.registry) {
      const e = Notifications.State.registry[id];
      if (e.command) byCommand[e.command] = e;
    }
    const found = {};
    for (const w of Object.values(Shell.NiriMsg._windows)) {
      if (w.appId !== "kitty" || !w.pid || w.pid in found) continue;
      found[w.pid] = null;
      const queue = [String(w.pid)];
      while (queue.length && !found[w.pid]) {
        const pid = queue.shift();
        if (pid !== String(w.pid) && byCommand[comm[pid]]) found[w.pid] = byCommand[comm[pid]];
        queue.push(...(children[pid] || []));
      }
    }
    if (JSON.stringify(found) !== JSON.stringify(root.agentPids)) root.agentPids = found;
  }

  ListModel { id: rows }

  // Titles and prompts are untrusted display text, never a command preview.
  function readable(value) {
    if (typeof value !== "string") return "";
    const text = value.replace(/\s+/g, " ").trim();
    if (!text || text.length > 240 || /[<>`{}|;$\\]|(?:^|\s)(?:--?\w|\/|~\/)|[a-f0-9]{16,}|[a-f0-9]{8}-[a-f0-9]{4}-/i.test(text)
        || /^(?:cd|env|exec|sudo|bash|sh|fish|python\d*|node|npm|npx|cargo|git|rg|ls|cat|codex|claude)(?:\s|$)/i.test(text)) return "";
    return text;
  }
  function context(value) {
    const text = readable(value);
    return /^(?:tmp|temp|scratch|worktree|new-chat|[a-f0-9]{6,})(?:[-_.\d].*)?$/i.test(text) ? "" : text;
  }
  // Keep source text in the tooltip; the narrow row gets a bounded excerpt.
  function shortTitle(value, fallback) {
    const text = readable(value);
    if (!text || /^(?:do (?:that|tht|it)|ok(?:ay)?|yes|no|continue|go ahead|thanks|hello|hi)[.!? ]*$/i.test(text)) return fallback;
    const sentence = text.split(/[\n.!?]/)[0].trim() || text;
    const words = sentence.split(/\s+/);
    let title = words.slice(0, 7).join(" ");
    if (title.length > 48) title = title.slice(0, 48).replace(/\s+\S*$/, "");
    return title + (title.length < sentence.length ? "…" : "");
  }
  function explanation(state, reason, terminal) {
    if (reason) return reason;
    return state === "ready" ? "Session opened · ready for a task"
        : state === "working" ? "In progress · no action needed"
        : state === "waiting" ? "Needs your attention · open session"
        : state === "done" ? (terminal ? "Activity ended · check session" : "Turn finished · ready to review")
        : state === "paused" ? "Paused · open session to continue"
        : state === "failed" ? "Could not finish · open for details"
        : state === "unknown" ? "No recent update · check session"
        : "Ready · no activity reported";
  }

  function refresh() {
    const now = Date.now();
    const list = [];
    let ticking = false;
    const raw = {};
    const windows = Shell.NiriMsg._windows;
    for (const id in windows) {
      const w = windows[id];
      if (w.appId !== "kitty") continue;
      const title = w.title || "";
      const match = Notifications.State.agentForTitle(title);
      // One kitty process can hold several windows; then only the title can say which is the agent.
      const shared = Object.values(windows).filter(o => o.pid === w.pid).length > 1;
      const running = shared ? null : root.agentPids[w.pid];
      if (!match && !running) { delete root.knownAgent[w.id]; continue; }
      let busy = Shell.NiriMsg._needsAttention(title) ? "waiting"
          : Shell.NiriMsg._isWorking(title) ? "working" : "";
      // A brief blank title between tools must not flash Done.
      if (!busy && root.lastRaw[w.id] === "working") {
        if (!root.settlingAt[w.id]) root.settlingAt[w.id] = now;
        if (now - root.settlingAt[w.id] < 1000) { busy = "working"; ticking = true; }
        else delete root.settlingAt[w.id];
      } else if (busy) delete root.settlingAt[w.id];
      raw[w.id] = busy;
      if (root.lastRaw[w.id] === "working" && busy === "") root.doneAt[w.id] = now;
      const inGrace = now - (root.doneAt[w.id] || 0) < root.doneGrace;
      if (inGrace) ticking = true;
      // Failures are keyed by niri window; older entries (and lookups that failed) by kitty pid.
      const failure = root.failed["win:" + w.id] || root.failed[w.pid];
      const state = failure ? (failure.state || "failed") : busy || ((root.doneAt[w.id] && Shell.NiriMsg._unread[w.id]) || inGrace ? "done" : "idle");
      if (match) root.knownAgent[w.id] = match.agent;
      const agent = root.knownAgent[w.id] || running;
      list.push({
        key: "win:" + w.id, reportJson: "", winId: w.id, app: "", link: "", state, leaving: false,
        name: agent ? agent.name : "Agent",
        icon: agent ? agent.mascot || "ghost.png" : "ghost.png",
        task: shortTitle(match ? context(title.replace(/^(\[\s*[!.]\s*\]|[⠀-⣿◐◑✳])\s*/, "")) : "", (agent ? agent.name : "Agent") + " session"),
        fullTask: title,
        detail: explanation(state, failure ? (failure.state === "paused" ? "Usage limit reached · open session" : "Could not finish · open for details") : "", true)
      });
    }
    root.lastRaw = raw;

    for (const sid in root.sessions) {
      const s = root.sessions[sid];
      const at = (s.time || 0) * 1000;
      let state = s.state, leaving = false;
      if ((state === "working" || state === "waiting" || state === "ready") && now - at > root.statusFreshness) state = "unknown";
      if (!["ready", "done", "working", "waiting", "paused", "failed", "unknown"].includes(state)) continue;
      const agent = Notifications.State.agent(s.agent) || {};
      const name = agent.name || s.agent || "Agent";
      list.push({ key: "session:" + sid, reportJson: JSON.stringify(s), winId: -1, app: s.app || "", link: s.open || "",
                  state, leaving, name, icon: agent.mascot || "ghost.png",
                  task: shortTitle(s.task, name + (context(s.project) ? " · " + context(s.project) : " session")),
                  fullTask: (readable(s.task) || "") + (context(s.project) ? "\nProject: " + context(s.project) : ""),
                  detail: explanation(state, state === "unknown" ? "" : readable(s.reason), false) });
    }
    // Distinguish legacy sessions that supplied no task without exposing IDs.
    const totals = {}, seen = {};
    for (const item of list) totals[item.task] = (totals[item.task] || 0) + 1;
    for (const item of list) {
      const title = item.task;
      if (totals[title] > 1) item.task += " · " + (seen[title] = (seen[title] || 0) + 1);
    }
    // Retain insertion order across status changes; new cards append.
    const positions = {};
    for (let i = 0; i < rows.count; i++) positions[rows.get(i).key] = i;
    list.sort((a, b) => (positions[a.key] ?? rows.count) - (positions[b.key] ?? rows.count));
    root.ticking = ticking;
    sync(list);
  }

  // Update rows in place so a row keeps its delegate (and only animates when it changes).
  function sync(list) {
    const keys = list.map(r => r.key);
    for (let i = rows.count - 1; i >= 0; i--)
      if (!keys.includes(rows.get(i).key)) rows.remove(i);
    list.forEach((r, i) => {
      let at = -1;
      for (let j = i; j < rows.count; j++) if (rows.get(j).key === r.key) { at = j; break; }
      if (at < 0) { rows.insert(i, r); return; }
      if (at !== i) rows.move(at, i, 1);
      const cur = rows.get(i);
      for (const k in r) if (cur[k] !== r[k]) rows.setProperty(i, k, r[k]);
    });
  }

  Component {
    id: dismissal
    Process {
      property string sessionId
      onExited: (code) => {
        if (code !== 0) console.warn("Could not dismiss agent card", sessionId);
        sessionFile.reload();
        destroy();
      }
    }
  }

  function dismiss(key, expectedReport) {
    if (!key.startsWith("session:")) return false;
    const sid = key.slice(8), report = root.sessions[sid];
    if (!report) return false;
    if (expectedReport && JSON.stringify(report) !== expectedReport) return false;
    const stale = Date.now() - (report.time || 0) * 1000 > root.statusFreshness;
    if ((report.state === "working" || report.state === "waiting") && !stale) return false;
    const process = dismissal.createObject(root, {
      sessionId: sid,
      command: ["toasty-dismiss", sid, expectedReport || JSON.stringify(report)]
    });
    process.running = true;
    return true;
  }

  IpcHandler {
    target: "toastyDock"
    function dismiss(sessionId: string): bool { return root.dismiss("session:" + sessionId, ""); }
    function dismissReport(sessionId: string, expectedReport: string): bool { return root.dismiss("session:" + sessionId, expectedReport); }
    function open(sessionId: string): bool {
      for (let i = 0; i < rows.count; i++) {
        const row = rows.get(i);
        if (row.key === "session:" + sessionId) { root.open(row); return true; }
      }
      return false;
    }
    function status(): string {
      const result = [];
      for (let i = 0; i < rows.count; i++) {
        const row = rows.get(i);
        result.push({key: row.key, state: row.state, task: row.task, detail: row.detail, reportJson: row.reportJson});
      }
      return JSON.stringify(result);
    }
  }

  function open(row) {
    if (row.key.startsWith("session:") && row.state === "done") dismiss(row.key, row.reportJson);
    if (row.link) Qt.openUrlExternally(row.link);
    else if (row.app) Shell.NiriMsg.focusApp(row.name, row.app);
    else Shell.NiriMsg.focusWindow(row.winId);
  }

  implicitWidth: Shell.Theme.lerp(Shell.Theme.sidebarCardExpandedWidth,
      Shell.Theme.sidebarCardCollapsedWidth, widthProgress)
  implicitHeight: rows.count === 0 ? 0 : rows.count * rowHeight + 8
  Behavior on implicitHeight { NumberAnimation { duration: 240; easing.type: Easing.OutCubic } }
  visible: implicitHeight > 0
  radius: Shell.Theme.lerp(12, Shell.Theme.sidebarCardRadius, widthProgress)

  // Library status colours: Working neutral, Waiting orange, Done blue, Failed red.
  function pillColor(state) {
    return state === "ready" ? "#666666" : state === "waiting" ? "#FF8A00" : state === "done" ? "#2563EB"
        : state === "unknown" ? "#666666" : state === "paused" ? "#8B6F47" : state === "failed" ? "#DC2626" : state === "working" ? "#E4DFE6" : "transparent";
  }
  function pillInk(state) { return state === "working" ? "#211E24" : "white"; }
  function label(state) {
    return state === "ready" ? "Ready" : state === "waiting" ? "Needs you" : state === "done" ? "Done"
        : state === "unknown" ? "Unknown" : state === "paused" ? "Paused" : state === "failed" ? "Failed" : state === "working" ? "Working" : "Idle";
  }

  Repeater {
    model: rows
    delegate: Item {
      id: row
      required property int index
      required property string key
      required property string reportJson
      required property int winId
      required property string app
      required property string link
      required property string state
      required property bool leaving
      required property string name
      required property string icon
      required property string fullTask
      required property string task
      required property string detail
      readonly property string status: state
      readonly property bool dismissible: key.startsWith("session:") && state !== "working" && state !== "waiting"
      y: 4 + index * root.rowHeight
      Behavior on y { NumberAnimation { duration: 240; easing.type: Easing.OutCubic } }
      width: root.width
      height: root.rowHeight
      opacity: leaving ? 0 : 1
      Behavior on opacity { NumberAnimation { duration: root.leaveDuration; easing.type: Easing.OutCubic } }
      activeFocusOnTab: true
      Keys.onReturnPressed: root.open(row)
      Keys.onSpacePressed: root.open(row)
      Accessible.role: Accessible.Button
      Accessible.name: name + ", " + root.label(status) + ": " + task + ". " + detail
      Accessible.onPressAction: root.open(row)

      // The library's Done moment: pill pops, mascot hops, check springs in.
      // The Done pop is skipped when reduced motion is on; the pill still turns blue.
      onStatusChanged: if (status === "done" && winId < 0 && !Notifications.State.reducedMotion) celebrate.restart()
      Component.onCompleted: if (status === "done" && winId < 0 && !Notifications.State.reducedMotion) celebrate.restart()
      SequentialAnimation {
        id: celebrate
        ParallelAnimation {
          NumberAnimation { target: pillLift; property: "y"; to: -6; duration: 180; easing.type: Easing.OutCubic }
          NumberAnimation { target: pill; property: "scale"; to: 1.06; duration: 180; easing.type: Easing.OutCubic }
          NumberAnimation { target: hop; property: "y"; to: -8; duration: 180; easing.type: Easing.OutCubic }
        }
        ParallelAnimation {
          NumberAnimation { target: pillLift; property: "y"; to: 1; duration: 180; easing.type: Easing.InOutCubic }
          NumberAnimation { target: pill; property: "scale"; to: 0.98; duration: 180; easing.type: Easing.InOutCubic }
          NumberAnimation { target: hop; property: "y"; to: 0; duration: 220; easing.type: Easing.OutBack }
        }
        ParallelAnimation {
          NumberAnimation { target: pillLift; property: "y"; to: 0; duration: 160; easing.type: Easing.OutCubic }
          NumberAnimation { target: pill; property: "scale"; to: 1; duration: 160; easing.type: Easing.OutCubic }
        }
      }

      HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
      ToolTip {
        id: detailTip
        popupType: Popup.Window
        x: row.width + 8
        y: 0
        visible: hover.hovered || row.activeFocus || dismissButton.activeFocus
        delay: 500
        timeout: 0
        width: 300
        padding: 14
        text: row.name + "\n" + (row.fullTask || row.task) + "\n" + row.detail
        contentItem: Text {
          text: detailTip.text
          textFormat: Text.PlainText
          color: "#F0EDF2"
          font.family: Shell.Theme.fontFamily
          font.pixelSize: 12
          wrapMode: Text.Wrap
        }
        background: Rectangle { color: "#29272D"; radius: 10; border.color: "#66606E" }
      }

      MouseArea {
        anchors.fill: parent
        property string pressedKey
        property string pressedReport
        onPressed: { pressedKey = row.key; pressedReport = row.reportJson; }
        onClicked: if (pressedKey === row.key && pressedReport === row.reportJson) root.open(row)
      }

      Rectangle {
        id: dismissButton
        z: 2
        visible: row.dismissible
        opacity: hover.hovered || activeFocus ? 1 : 0
        activeFocusOnTab: row.dismissible
        Behavior on opacity {
          NumberAnimation { duration: Notifications.State.reducedMotion ? 0 : 120 }
        }
        anchors.right: parent.right
        anchors.rightMargin: 4
        y: 0
        width: 26; height: 26; radius: 6
        color: dismissArea.pressed ? Qt.rgba(1, 1, 1, 0.20)
            : dismissArea.containsMouse || activeFocus ? Qt.rgba(1, 1, 1, 0.10) : "transparent"
        border.width: activeFocus ? 1 : 0
        border.color: Qt.rgba(1, 1, 1, 0.5)
        Keys.onPressed: event => {
          if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space || event.key === Qt.Key_Delete) {
            root.dismiss(row.key, row.reportJson);
            event.accepted = true;
          }
        }
        Accessible.role: Accessible.Button
        Accessible.name: "Dismiss " + row.name + " card"
        Accessible.onPressAction: root.dismiss(row.key, row.reportJson)
        Text {
          anchors.centerIn: parent
          text: "×"
          color: "#D8D2DC"
          font.family: Shell.Theme.fontFamily
          font.pixelSize: 17
        }
        MouseArea {
          id: dismissArea
          enabled: hover.hovered || dismissButton.activeFocus
          anchors.fill: parent
          hoverEnabled: true
          cursorShape: Qt.PointingHandCursor
          property string pressedKey
          property string pressedReport
          onPressed: { pressedKey = row.key; pressedReport = row.reportJson; }
          onClicked: if (pressedKey === row.key) root.dismiss(pressedKey, pressedReport)
        }
      }

      Rectangle {
        x: 4; y: 2
        width: parent.width - 8; height: parent.height - 4
        radius: 10
        color: Shell.Theme.daylight ? Shell.Theme.sidebarHoverFill : "#262626"
        opacity: hover.hovered ? 1 : 0
        Behavior on opacity { Shell.HoverAnimation {} }
      }

      // Mascot: centred when collapsed, with a status dot standing in for the pill.
      Image {
        id: mascot
        x: Shell.Theme.lerp(10, (root.width - width) / 2, root.widthProgress)
        anchors.verticalCenter: parent.verticalCenter
        width: 28; height: 28
        transform: Translate { id: hop }
        source: Qt.resolvedUrl("../assets/agents/" + row.icon)
        sourceSize: Qt.size(56, 56)
        fillMode: Image.PreserveAspectFit
        opacity: row.status === "idle" ? 0.55 : 1
        Behavior on opacity { NumberAnimation { duration: 240 } }
      }
      Rectangle {
        visible: row.status !== "idle"
        opacity: root.widthProgress
        x: mascot.x + mascot.width - 8; y: mascot.y + mascot.height - 8
        width: 10; height: 10; radius: 5
        color: row.status === "working" ? "#CBA6F7" : root.pillColor(row.status)
        border.width: 2; border.color: "#1E1E1E"
      }

      Column {
        x: 44
        anchors.verticalCenter: parent.verticalCenter
        width: parent.width - x - 8
        spacing: 2
        opacity: 1 - Shell.Theme.ramp(root.collapseProgress, 0, 0.3)
        visible: opacity > 0

        // The pill: label + icon, 20px tall.
        Rectangle {
          id: pill
          height: 20; radius: 10
          width: pillRow.width + 16
          transform: Translate { id: pillLift }
          color: root.pillColor(row.status)
          Behavior on color { ColorAnimation { duration: 240 } }
          border.width: row.status === "idle" ? 1 : 0
          border.color: "#AAA2AF"
          Row {
            id: pillRow
            x: 8; anchors.verticalCenter: parent.verticalCenter
            spacing: 4
            Text {
              anchors.verticalCenter: parent.verticalCenter
              text: row.winId >= 0 && row.status === "done" ? "Ended" : root.label(row.status)
              color: row.status === "idle" ? "#5C5760" : root.pillInk(row.status)
              font.family: Shell.Theme.fontFamily; font.pixelSize: 11; font.weight: Font.DemiBold
            }
            StatusIcon {
              id: icon
              anchors.verticalCenter: parent.verticalCenter
              status: row.status
              ink: root.pillInk(row.status)
            }
          }
        }
        Text {
          width: parent.width
          text: row.task
          textFormat: Text.PlainText
          wrapMode: Text.Wrap
          maximumLineCount: 2
          elide: Text.ElideRight
          color: "#F0EDF2"
          font.family: Shell.Theme.fontFamily; font.pixelSize: 11
        }
      }
    }
  }

  // Spinner while working, three dots while waiting, a green check badge when done, "!" when failed.
  component StatusIcon: Item {
    id: statusIcon
    property string status
    property color ink
    visible: status !== "idle"
    width: status === "done" ? 14 : 10; height: width

    Item {
      anchors.fill: parent
      visible: statusIcon.status === "working"
      Rectangle {
        anchors.fill: parent; radius: 5
        color: "transparent"; border.width: 1.5; border.color: Qt.rgba(0, 0, 0, 0.18)
      }
      Rectangle {
        width: 4; height: 4; radius: 2; x: 3; y: -0.5
        color: statusIcon.ink
      }
      RotationAnimator on rotation {
        from: 0; to: 360; duration: 900; loops: Animation.Infinite
        running: parent.visible
      }
    }
    Row {
      visible: statusIcon.status === "waiting"
      anchors.verticalCenter: parent.verticalCenter
      spacing: 1.5
      Repeater {
        model: 3
        Rectangle {
          required property int index
          width: 2.5; height: 2.5; radius: 1.25; color: "white"
          SequentialAnimation on opacity {
            running: statusIcon.status === "waiting"; loops: Animation.Infinite
            PauseAnimation { duration: index * 160 }
            NumberAnimation { from: 0.3; to: 1; duration: 320 }
            NumberAnimation { from: 1; to: 0.3; duration: 320 }
            PauseAnimation { duration: (2 - index) * 160 }
          }
        }
      }
    }
    Row {
      visible: statusIcon.status === "paused"
      anchors.centerIn: parent
      spacing: 2
      Repeater {
        model: 2
        Rectangle { width: 2; height: 7; radius: 0.5; color: statusIcon.ink }
      }
    }
    // Library check: green badge with a white ring, springing in 220ms after the pop starts.
    Rectangle {
      id: badge
      visible: statusIcon.status === "done"
      anchors.centerIn: parent
      width: 14; height: 14; radius: 7
      color: "#16A34A"
      border.width: 1.5; border.color: "white"
      scale: 0
      Text {
        anchors.centerIn: parent
        text: "✓"; color: "white"
        font.pixelSize: 9; font.weight: Font.Bold
      }
      SequentialAnimation {
        running: badge.visible
        PropertyAction { target: badge; property: "scale"; value: 0 }
        PauseAnimation { duration: 220 }
        NumberAnimation { target: badge; property: "scale"; to: 1; duration: 360; easing.type: Easing.OutBack; easing.overshoot: 2.2 }
      }
    }
    Text {
      visible: statusIcon.status === "failed" || statusIcon.status === "unknown"
      anchors.centerIn: parent
      text: statusIcon.status === "unknown" ? "?" : "!"; color: "white"
      font.pixelSize: 10; font.weight: Font.Bold
    }
  }
}
