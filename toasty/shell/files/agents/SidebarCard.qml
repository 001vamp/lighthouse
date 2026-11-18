import QtQuick
import QtQuick.Controls
import QtQuick.Shapes
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
  property real collapseProgress: 0
  readonly property real widthProgress: Shell.Theme.collapseWidth(collapseProgress)
  readonly property real heightProgress: Shell.Theme.collapseHeight(collapseProgress)
  readonly property int rowHeight: Math.round(Shell.Theme.lerp(58, 40, heightProgress))
  // "Jobs" and "Agents" headings, when the dock holds both: the row index each one sits above.
  // They fold away with the dock.
  property var headerAt: []
  readonly property int headerHeight: Math.round(Shell.Theme.lerp(22, 0, heightProgress))
  function headersAbove(i) { return headerAt.filter(h => h <= i).length; }
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
        key: "win:" + w.id, reportJson: "", winId: w.id, app: "", link: "", state, leaving: false, kind: "", count: "", stale: false, byIcon: "",
        name: agent ? agent.name : "Agent",
        icon: agent ? agent.mascot || "ghost.png" : "ghost.png",
        task: shortTitle(match ? context(title.replace(/^(\[\s*[!.]\s*\]|[⠀-⣿◐◑✳])\s*/, "")) : "", (agent ? agent.name : "Agent") + " session"),
        fullTask: title,
        detail: explanation(state, failure ? (failure.state === "paused" ? "Usage limit reached · open session" : "Could not finish · open for details") : "", true)
      });
    }
    root.lastRaw = raw;

    // Sessions an orchestrator spawned (T3 Code, today) fold into one job row per orchestrator
    // thread. The orchestrator stays an agent row of its own; the job is only what it started,
    // and says who started it. Only children that need you get a row, right under the job.
    const jobs = {};
    const owners = {};
    for (const sid in root.sessions) {
      const t3 = root.sessions[sid].t3 || {};
      if (t3.job && t3.job.id) (jobs[t3.job.id] = jobs[t3.job.id] || { job: t3.job, children: [] }).children.push(sid);
    }
    for (const sid in root.sessions) {
      const t3 = root.sessions[sid].t3 || {};
      if (t3.thread && jobs[t3.thread]) owners[t3.thread] = sid;
    }
    const sessionRows = {};
    for (const sid in root.sessions) {
      const s = root.sessions[sid];
      const at = (s.time || 0) * 1000;
      let state = s.state, leaving = false;
      // Needs you stays until the session says otherwise: going quiet doesn't answer it.
      if ((state === "working" || state === "ready") && now - at > root.statusFreshness) state = "unknown";
      if (!["ready", "done", "working", "waiting", "paused", "failed", "unknown"].includes(state)) continue;
      const agent = Notifications.State.agent(s.agent) || {};
      const name = agent.name || s.agent || "Agent";
      const job = (s.t3 || {}).job;
      sessionRows[sid] = { key: "session:" + sid, reportJson: JSON.stringify(s), winId: -1, app: s.app || "", link: s.open || "",
                  state, leaving, name, icon: agent.mascot || "ghost.png", byIcon: "", kind: job && job.id ? "child" : "", count: "",
                  stale: state === "waiting" && now - at > root.statusFreshness,
                  task: shortTitle(s.task, name + (context(s.project) ? " · " + context(s.project) : " session")),
                  fullTask: (readable(s.task) || "") + (context(s.project) ? "\nProject: " + context(s.project) : ""),
                  detail: state === "waiting" && now - at > root.statusFreshness ? "Needs you · no update for 10+ min · open session"
                      : explanation(state, state === "unknown" ? "" : readable(s.reason), false) };
    }
    const childRows = {};
    for (const id in jobs) {
      const members = jobs[id].children.filter(sid => sessionRows[sid]);
      const owner = sessionRows[owners[id]];
      const all = members.map(sid => sessionRows[sid]);
      if (!members.length) continue;
      const states = all.map(r => r.state);
      const has = st => states.includes(st);
      // Anything that needs you outranks progress, so a failure never hides behind "Working".
      const state = has("waiting") ? "waiting" : has("failed") ? "failed" : has("paused") ? "paused"
          : has("working") ? "working" : has("unknown") ? "unknown" : has("ready") ? "ready" : "done";
      const waiting = members.filter(sid => sessionRows[sid].state === "waiting").length;
      const finished = members.filter(sid => sessionRows[sid].state === "done").length;
      const reports = {};
      for (const sid of members) reports[sid] = root.sessions[sid];
      const title = shortTitle(jobs[id].job.title, "Agent job");
      const starter = owner ? owner.name : "";
      list.push({ key: "job:" + id, reportJson: JSON.stringify(reports), winId: -1, app: "", link: jobs[id].job.open || (owner ? owner.link : ""),
                  state, leaving: false, name: title, icon: sessionRows[members[0]].icon, byIcon: owner ? owner.icon : "",
                  kind: "job", stale: all.every(r => r.state !== "waiting" || r.stale), count: members.some(sid => sessionRows[sid].state === "working") ? finished + "/" + members.length : "",
                  task: title,
                  fullTask: (starter ? "Started by " + starter + " in “" : "Started in “") + (readable(jobs[id].job.title) || "Agent job") + "”\n" + members.length + (members.length === 1 ? " agent: " : " agents: ")
                      + members.map(sid => sessionRows[sid].name + " " + root.label(sessionRows[sid].state).toLowerCase()).join(", "),
                  detail: finished + " of " + members.length + " agents finished" + (waiting ? " · " + waiting + (waiting === 1 ? " needs you" : " need you") : "") });
      childRows["job:" + id] = members.filter(sid => ["waiting", "failed", "paused"].includes(sessionRows[sid].state)).map(sid => sessionRows[sid]);
      for (const sid of members) delete sessionRows[sid];
    }
    for (const sid in sessionRows) list.push(sessionRows[sid]);
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
    // Jobs first, each with its children that need you right under it; then single agents.
    const jobRows = [], agentRows = [];
    for (const item of list) {
      if (item.kind === "job") jobRows.push(item, ...(childRows[item.key] || []));
      else agentRows.push(item);
    }
    root.ticking = ticking;
    sync(jobRows.concat(agentRows));
    const headers = jobRows.length && agentRows.length ? [0, jobRows.length] : [];
    if (JSON.stringify(headers) !== JSON.stringify(root.headerAt)) root.headerAt = headers;
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
    if (key.startsWith("job:")) {
      let reports = {};
      try { reports = JSON.parse(expectedReport || "{}"); } catch (e) {}
      let any = false;
      for (const sid in reports) any = dismiss("session:" + sid, JSON.stringify(reports[sid])) || any;
      return any;
    }
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
    if ((row.key.startsWith("session:") || row.key.startsWith("job:")) && row.state === "done") dismiss(row.key, row.reportJson);
    if (row.link) Qt.openUrlExternally(row.link);
    else if (row.app) Shell.NiriMsg.focusApp(row.name, row.app);
    else Shell.NiriMsg.focusWindow(row.winId);
  }

  implicitWidth: Shell.Theme.lerp(Shell.Theme.sidebarCardExpandedWidth,
      Shell.Theme.sidebarCardCollapsedWidth, widthProgress)
  implicitHeight: rows.count === 0 ? 0 : rows.count * rowHeight + headerAt.length * headerHeight + 8
  Behavior on implicitHeight { NumberAnimation { duration: Notifications.State.reducedMotion ? 0 : 240; easing.type: Easing.OutCubic } }
  visible: implicitHeight > 0
  radius: Shell.Theme.lerp(12, Shell.Theme.sidebarCardRadius, widthProgress)

  // Library status colours: Working neutral, Waiting orange, Done blue, Failed red.
  function pillColor(state) {
    return state === "ready" ? "#666666" : state === "waiting" ? "#FF8A00" : state === "done" ? "#2563EB"
        : state === "unknown" ? "#666666" : state === "paused" ? "#8B6F47" : state === "failed" ? "#DC2626" : state === "working" ? "#E4DFE6" : "transparent";
  }
  // Dark ink on the light Working pill and on orange, where white is too faint to read.
  function pillInk(state) { return state === "working" || state === "waiting" ? "#211E24" : "white"; }
  function label(state) {
    return state === "ready" ? "Ready" : state === "waiting" ? "Needs you" : state === "done" ? "Done"
        : state === "unknown" ? "Unknown" : state === "paused" ? "Paused" : state === "failed" ? "Failed" : state === "working" ? "Working" : "Idle";
  }

  Repeater {
    model: root.headerAt
    Text {
      required property int modelData
      required property int index
      x: 14
      y: 4 + modelData * root.rowHeight + index * root.headerHeight + (root.headerHeight - height) / 2 + 2
      text: index === 0 ? "Jobs" : "Agents"
      color: Qt.rgba(1, 1, 1, 0.45)
      font.family: Shell.Theme.fontFamily; font.pixelSize: 10; font.weight: Font.DemiBold
      font.capitalization: Font.AllUppercase; font.letterSpacing: 0.6
      opacity: 1 - Shell.Theme.ramp(root.collapseProgress, 0, 0.3)
      visible: opacity > 0
    }
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
      required property string byIcon  // a job's: the mascot of the agent that started it
      required property string fullTask
      required property string task
      required property string detail
      required property string kind    // "job": an orchestrator's agents; "child": one of them
      required property string count   // a job's finished/total agents
      required property bool stale     // Needs you, but the session has been quiet for a while
      readonly property string status: state
      readonly property bool dismissible: (key.startsWith("session:") || kind === "job") && state !== "working" && (state !== "waiting" || stale)
      // Children sit a step in, under their job.
      readonly property int indent: kind === "child" ? Math.round(14 * (1 - root.widthProgress)) : 0
      y: 4 + index * root.rowHeight + root.headersAbove(index) * root.headerHeight
      Behavior on y { NumberAnimation { duration: Notifications.State.reducedMotion ? 0 : 240; easing.type: Easing.OutCubic } }
      width: root.width
      height: root.rowHeight
      opacity: leaving ? 0 : 1
      Behavior on opacity { NumberAnimation { duration: root.leaveDuration; easing.type: Easing.OutCubic } }
      activeFocusOnTab: true
      Keys.onReturnPressed: root.open(row)
      Keys.onSpacePressed: root.open(row)
      Accessible.role: Accessible.Button
      readonly property string opens: kind === "job" ? "Open job: " : kind === "child" ? "Open agent thread: " : ""
      Accessible.name: opens + name + ", " + root.label(status) + ": " + task + ". " + detail
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
        text: row.opens + row.name + "\n" + (row.fullTask || row.task) + "\n" + row.detail
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

      // Hover and keyboard focus share the fill; focus also gets an inset ring.
      Rectangle {
        x: 4; y: 2
        width: parent.width - 8; height: parent.height - 4
        radius: 10
        color: Shell.Theme.daylight ? Shell.Theme.sidebarHoverFill : "#262626"
        opacity: hover.hovered || row.activeFocus ? 1 : 0
        Behavior on opacity { Shell.HoverAnimation {} }
        border.width: row.activeFocus ? 2 : 0
        border.color: Qt.rgba(1, 1, 1, 0.5)
      }

      // A job reads as a group, never as one agent: dashed outline, and its mascot on a stack.
      Shape {
        id: jobOutline
        // Stays when the dock collapses: that's when a job is easiest to mistake for an agent.
        visible: row.kind === "job"
        x: 4; y: 2
        width: parent.width - 8; height: parent.height - 4
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
          strokeColor: Qt.rgba(1, 1, 1, 0.22)
          strokeWidth: 1
          strokeStyle: ShapePath.DashLine
          dashPattern: [3, 3]
          fillColor: "transparent"
          PathRectangle { x: 0.5; y: 0.5; width: jobOutline.width - 1; height: jobOutline.height - 1; radius: 10 }
        }
      }
      Repeater {
        model: row.kind === "job" ? [0.07, 0.13] : []
        Rectangle {
          required property real modelData
          required property int index
          x: mascot.x + 2 + (2 - index) * 3; y: mascot.y + 2 - (2 - index) * 3
          width: 24; height: 24; radius: 7
          color: Qt.rgba(1, 1, 1, modelData)
        }
      }

      // Mascot: centred when collapsed, with a status dot standing in for the pill.
      Image {
        id: mascot
        x: Shell.Theme.lerp(10, (root.width - width) / 2, root.widthProgress) + row.indent
        anchors.verticalCenter: parent.verticalCenter
        width: 28; height: 28
        transform: Translate { id: hop }
        source: Qt.resolvedUrl("../assets/agents/" + row.icon)
        sourceSize: Qt.size(56, 56)
        fillMode: Image.PreserveAspectFit
        opacity: row.status === "idle" ? 0.55 : row.status === "paused" ? 0.7 : 1
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
        x: 44 + row.indent
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
              text: row.winId >= 0 && row.status === "done" ? "Ended"
                  : root.label(row.status) + (row.status === "working" && row.count ? " " + row.count : "")
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
        // A job says who started it: "from" and that agent's mascot, then the thread's title.
        Row {
          width: parent.width
          spacing: 4
          Text {
            id: fromLabel
            visible: row.byIcon !== ""
            text: "from"
            color: "#A9A3AD"
            font.family: Shell.Theme.fontFamily; font.pixelSize: 11
          }
          Image {
            id: fromIcon
            visible: row.byIcon !== ""
            y: 1
            width: 12; height: 12
            source: row.byIcon ? Qt.resolvedUrl("../assets/agents/" + row.byIcon) : ""
            sourceSize: Qt.size(24, 24)
            fillMode: Image.PreserveAspectFit
          }
          Text {
            width: parent.width - (fromLabel.visible ? fromLabel.implicitWidth + fromIcon.width + 8 : 0)
            text: row.task
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            maximumLineCount: row.byIcon !== "" ? 1 : 2
            elide: Text.ElideRight
            color: "#F0EDF2"
            font.family: Shell.Theme.fontFamily; font.pixelSize: 11
          }
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
        running: parent.visible && !Notifications.State.reducedMotion
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
          width: 2.5; height: 2.5; radius: 1.25; color: statusIcon.ink
          SequentialAnimation on opacity {
            running: statusIcon.status === "waiting" && !Notifications.State.reducedMotion; loops: Animation.Infinite
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
      scale: Notifications.State.reducedMotion ? 1 : 0
      Text {
        anchors.centerIn: parent
        text: "✓"; color: "white"
        font.pixelSize: 9; font.weight: Font.Bold
      }
      SequentialAnimation {
        running: badge.visible && !Notifications.State.reducedMotion
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
