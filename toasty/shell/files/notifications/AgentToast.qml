import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Widgets
import "." as Notifications
import ".." as Shell

// Agent toast, built to the Pond toast library spec (outputs/pond-toast-library).
// Layout follows each layout's "Use for" rule:
//   reply field          → 07 inset, 440px, 40px icon slot
//   needs / failed       → 02 floating left, mascot 8px above the text column
//   done / info, long    → 03 sitting centre, mascot sits 8px onto the card
//   done / info          → 01 corner companion, mascot overlaps the top-left corner
//   away summary         → 03 sitting, one line per agent event while you were away
// 4px grid: padding 16 top/bottom, 20 left, 48 right (clears the close button).
Item {
  id: root
  property var notification: ({})
  readonly property var identity: Notifications.State.agentInfo(notification) || ({name: "Agent", provider: "", model: ""})
  readonly property string agentKey: String(identity.name || "").toLowerCase().replace(/[^a-z0-9]+/g, "")
  readonly property var look: lookFor(agentKey)
  readonly property string iconSource: {
    if (identity.icon) return identity.icon.startsWith("/") ? "file://" + identity.icon
      : Quickshell.iconPath(identity.icon, "");
    return Qt.resolvedUrl("../assets/agents/" + look.icon);
  }
  readonly property var actions: (notification.actions || [])
      .filter(a => a.identifier !== "default" && a.identifier !== "inline-reply")
  readonly property var suggestions: notification.suggestions || []
  // toasty-notify proves it's still listening (and that a reply landed) with marker files.
  // Set from onNotificationChanged, where bindings on `notification` may not have updated yet.
  property bool usesMarkers: false
  property string markerPath: ""
  property bool listening: true
  readonly property bool hasReply: !!notification.hasInlineReply && listening
  readonly property bool canReply: hasReply && !sent
  readonly property string kind: notification.kind || (notification.urgency === 2 ? "needs" : "info")
  readonly property bool away: kind === "away"
  readonly property bool sticky: kind === "needs" || kind === "failed" || away
  // Sticky toasts carry a lasting edge in their status colour; Done stays light.
  readonly property color edge: kind === "needs" ? "#FF8A00" : kind === "failed" ? "#DC2626" : "transparent"
  // How it ended, shown for a moment before it leaves ("Allowed", "Denied", "Handled in the session").
  readonly property string stamp: Notifications.State.stamps[notification.id] || ""
  readonly property bool motion: !Notifications.State.reducedMotion
  // A toast that carries code (e.g. a command to approve) reads as: question, code, caption.
  readonly property string code: notification.code || ""
  readonly property string messageText: code ? String(notification.summary || "")
                                             : Notifications.State.message(notification)
  readonly property string caption: {
    const lines = [code ? String(notification.body || "") : ""];
    const at = code ? Notifications.State.allowedAt[code] : 0;
    // Asked again for something you just allowed: say so, so it doesn't read as new.
    if (at && at < (notification.timestamp || 0) && Date.now() - at < 30 * 60000) {
      const minutes = Math.floor((Date.now() - at) / 60000);
      lines.push("You allowed this " + (minutes < 1 ? "just now." : minutes + " min ago."));
    }
    return lines.filter(v => v).join("\n");
  }
  readonly property bool held: hover.hovered || field.activeFocus || field.text.length > 0
  property string delivery: ""   // "", "sending", "sent", "failed"
  readonly property bool sent: delivery !== ""
  property real remaining: 1

  readonly property string layout: away ? "sitting"
      : hasReply ? "inset"
      : sticky ? "floating"
      : messageText.length > 90 ? "sitting" : "corner"

  // Library tokens (light toast).
  readonly property color surface: "#E4DFE6"
  readonly property color ink: "#211E24"
  readonly property color muted: "#5C5760"
  readonly property color line: Qt.rgba(0, 0, 0, 0.14)
  readonly property color hoverFill: Qt.rgba(0, 0, 0, 0.07)
  readonly property color focusRing: "#CBA6F7"

  // Geometry. pad leaves room for the shadow; insets leave room for mascots that stick out.
  readonly property int pad: 12
  readonly property int mascotSize: layout === "inset" ? 40 : 48
  readonly property int leftInset: layout === "corner" ? 16 : 0
  readonly property int topInset: layout === "corner" ? 20 : layout === "floating" ? mascotSize + 8
      : layout === "sitting" ? mascotSize - 8 : 0
  readonly property int cardWidth: layout === "inset" ? 440 : 384
  readonly property int padLeft: layout === "corner" ? 44 : layout === "inset" ? 68 : 20
  readonly property int cardHeight: Math.max(72, body.height + 32)

  signal activated
  signal dismissed
  signal expired
  signal replied(string text)
  signal actionInvoked(string identifier)

  // Accent and mascot come from the agent registry (agents.json); unknown agents get the grey ghost.
  function lookFor(key) {
    const e = Notifications.State.agent(key);
    return e ? { accent: e.accent || "#8A8590", icon: e.mascot || "ghost.png", avatar: !!e.avatar }
             : { accent: "#8A8590", icon: "ghost.png" };
  }

  function send(text) {
    text = String(text || "").trim();
    if (!text || sent) return;
    delivery = "sending";
    field.focus = false;
    root.replied(text);
    if (usesMarkers) {
      confirm.forId = shownId;
      confirm.running = true;
    } else {
      delivery = "sent";
      sentClose.forId = shownId;
      sentClose.start();
    }
  }

  // The same notification comes back as a fresh object whenever its text or icon changes;
  // only a different notification should clear what you're typing or a reply in flight.
  property string shownId: ""
  onNotificationChanged: {
    if ((notification || {}).id === shownId) return;
    shownId = (notification || {}).id || "";
    // Anything still in flight belongs to the previous notification.
    probe.running = false;
    confirm.running = false;
    sentClose.stop();
    delivery = "";
    pickedUp = false;
    pickup.stop();
    listening = true;
    field.text = "";
    remaining = 1;
    shownAt = Date.now();
    if (countdown.running) countdown.restart();
    const n = notification || {};
    usesMarkers = /^(Toasty|PondAgent)\|/.test(String(n.appName || "")) && !!n.serverId;
    markerPath = Notifications.State.markerDir + "/" + (n.serverId || 0);
    if (motion) {
      if (kind === "done") hop.restart();
      else if (kind === "failed") nudge.restart();
    }
    // Buttons and replies only mean something while the sender is still listening.
    if (usesMarkers && (n.hasInlineReply || (n.actions || []).some(a => a.identifier !== "default"
                                                                    && a.identifier !== "inline-reply"))) {
      probe.forId = shownId;
      probe.running = true;
    }
  }

  // Wait up to 1.5s for a marker: the sender writes it just after Notify returns,
  // and after a reply has been typed into the agent's terminal.
  readonly property var waitFor: ["sh", "-c",
      'for i in $(seq 15); do [ -e "$1" ] && exit 0; sleep 0.1; done; exit 1', "sh"]
  // Each check remembers which notification it was started for, and its result is
  // dropped if the toast has moved on to another one since.
  Process {
    id: probe
    property string forId: ""
    command: Notifications.State.aliveCheckCommand.concat([root.markerPath + ".alive"])
    onExited: code => { if (forId === root.shownId) root.listening = code === 0; }
  }
  Process {
    id: confirm
    property string forId: ""
    command: root.waitFor.concat([root.markerPath + ".delivered"])
    onExited: code => {
      if (forId !== root.shownId) return;
      root.delivery = code === 0 ? "sent" : "failed";
      if (code === 0) { pickup.forId = forId; pickup.since = Date.now(); pickup.start(); }
    }
  }

  implicitWidth: pad * 2 + leftInset + cardWidth
  implicitHeight: pad * 2 + topInset + cardHeight

  HoverHandler { id: hover }

  Item {
    id: card
    transform: Translate { id: cardShift }
    x: root.pad + root.leftInset
    y: root.pad + root.topInset
    width: root.cardWidth
    height: root.cardHeight

    Shell.Shadow {
      anchors.fill: parent
      cornerRadius: 20
      shadows: [{ x: 0, y: 1, blur: 2, alpha: 0.25 }, { x: 0, y: 8, blur: 24, alpha: 0.35 }]
    }
    Rectangle {
      anchors.fill: parent
      radius: 20
      color: root.surface
      antialiasing: true
      border.width: root.sticky && !root.away ? 2 : 0
      border.color: root.edge
    }

    // Clicking anywhere that isn't a control opens the agent's session.
    MouseArea {
      anchors.fill: parent
      enabled: !root.away
      cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
      onClicked: root.activated()
    }

    Column {
      id: body
      x: root.padLeft
      y: root.layout === "inset" ? 16 : Math.round((root.cardHeight - height) / 2)
      width: root.cardWidth - x - 48

      Item {
        width: parent.width; height: 20
        Row {
          spacing: 8
          anchors.verticalCenter: parent.verticalCenter
          Rectangle {
            width: 8; height: 8; radius: 4
            anchors.verticalCenter: parent.verticalCenter
            color: root.look.accent
          }
          Text {
            width: Math.max(0, Math.min(implicitWidth, time.x - 16 - 8))
            text: root.identity.name
            color: root.ink; font.family: Shell.Theme.fontFamily
            font.pixelSize: 13; font.weight: Font.Bold; elide: Text.ElideRight
          }
        }
        Text {
          id: time
          // At most 60% of the row: a long project name gets cut from the left, so the
          // time and waiting count stay readable and the sender keeps its space.
          width: Math.min(implicitWidth, parent.width * 0.6)
          elide: Text.ElideLeft
          x: parent.width - width
          anchors.verticalCenter: parent.verticalCenter
          readonly property string age: Notifications.State.relativeTimeLabel(root.notification.timestamp || Date.now())
          // Brought back after a shell reload: say how long it has been waiting.
          text: [root.notification.context,
                 root.notification.restored ? "Still waiting" + (age === "Now" ? "" : " · " + age) : age,
                 Notifications.State.queue.length ? Notifications.State.queue.length + " waiting" : ""]
              .filter(v => v).join(" · ")
          color: root.muted; font.family: Shell.Theme.fontFamily; font.pixelSize: 11
        }
      }
      Item { width: 1; height: 2 }
      Text {
        width: parent.width
        text: root.messageText
        color: root.ink; font.family: Shell.Theme.fontFamily
        font.pixelSize: 13; lineHeight: 20; lineHeightMode: Text.FixedHeight
        wrapMode: Text.Wrap; maximumLineCount: 4; elide: Text.ElideRight
      }

      // While you were away: one line per event, Waiting first. Each opens its agent;
      // the card goes once every line is opened, or with the close button.
      Item { width: 1; height: 8; visible: root.away }
      Column {
        visible: root.away
        width: parent.width
        spacing: 4
        Repeater {
          model: root.away ? root.notification.lines || [] : []
          delegate: Rectangle {
            id: awayLine
            required property var modelData
            readonly property bool opened: (root.notification.opened || []).includes(modelData.id)
            width: parent.width; height: 32; radius: 8
            color: lineArea.containsMouse || activeFocus ? root.hoverFill : "transparent"
            opacity: opened ? 0.5 : 1
            activeFocusOnTab: true
            border.width: activeFocus ? 2 : 0
            border.color: root.focusRing
            Accessible.role: Accessible.Button
            Accessible.name: modelData.name + ", " + modelData.kind + ": " + modelData.text
            Accessible.onPressAction: Notifications.State.openLine(root.notification, modelData)
            Keys.onReturnPressed: Notifications.State.openLine(root.notification, modelData)
            Keys.onSpacePressed: Notifications.State.openLine(root.notification, modelData)
            Rectangle {
              x: 8; anchors.verticalCenter: parent.verticalCenter
              width: 8; height: 8; radius: 4
              color: awayLine.modelData.kind === "needs" ? "#FF8A00"
                   : awayLine.modelData.kind === "failed" ? "#DC2626" : "#2563EB"
            }
            Text {
              x: 24; width: parent.width - 32
              anchors.verticalCenter: parent.verticalCenter
              text: "<b>" + awayLine.modelData.name + "</b>"
                    + (awayLine.modelData.context ? " · " + awayLine.modelData.context : "")
                    + " — " + (awayLine.modelData.kind === "needs" ? "Waiting"
                               : awayLine.modelData.kind === "failed" ? "Failed" : "Done")
                    + ": " + awayLine.modelData.text
              textFormat: Text.StyledText
              color: root.ink; font.family: Shell.Theme.fontFamily; font.pixelSize: 12
              elide: Text.ElideRight; maximumLineCount: 1
            }
            MouseArea {
              id: lineArea
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: Notifications.State.openLine(root.notification, awayLine.modelData)
            }
          }
        }
      }

      // Code block: 8px below the question, monospace on a faint fill, up to 3 lines.
      Item { width: 1; height: 8; visible: root.code !== "" }
      Rectangle {
        visible: root.code !== ""
        width: parent.width
        height: codeText.height + 16
        radius: 8
        color: root.hoverFill
        Text {
          id: codeText
          x: 12; y: 8
          width: parent.width - 24
          text: root.code
          color: root.ink; font.family: Shell.Theme.plexFontFamily
          font.pixelSize: 12; lineHeight: 18; lineHeightMode: Text.FixedHeight
          wrapMode: Text.Wrap; maximumLineCount: 3; elide: Text.ElideRight
        }
      }
      // Never let a cut-off command pass as the whole thing you're approving.
      readonly property string fullCaption: [codeText.truncated ? "Cut off: open the session to see all of it." : "",
                                             root.caption].filter(v => v).join("\n")
      Item { width: 1; height: 8; visible: body.fullCaption !== "" }
      Text {
        visible: body.fullCaption !== ""
        width: parent.width
        text: body.fullCaption
        color: root.muted; font.family: Shell.Theme.fontFamily
        font.pixelSize: 12; lineHeight: 16; lineHeightMode: Text.FixedHeight
        wrapMode: Text.Wrap; maximumLineCount: 3; elide: Text.ElideRight
      }

      // Actions: 12px below the message, first one primary; wraps if they don't fit.
      Item { width: 1; height: 12; visible: actionRow.visible }
      Flow {
        id: actionRow
        width: parent.width; spacing: 8
        visible: root.actions.length > 0 && !root.sent && !root.stamp && root.listening
        Repeater {
          model: root.actions
          delegate: Notifications.AgentButton {
            required property var modelData
            required property int index
            label: modelData.text
            // Allow and Deny carry equal weight; "Always allow" is the quieter choice.
            primary: index === 0 || modelData.identifier === "deny"
            ink: root.ink; surface: root.surface; line: root.line; hoverFill: root.hoverFill
            onClicked: root.actionInvoked(modelData.identifier)
          }
        }
      }

      // How it ended, or why the buttons went away.
      Item { width: 1; height: 12; visible: stampRow.visible }
      Row {
        id: stampRow
        visible: root.stamp !== "" || (root.usesMarkers && !root.listening && root.actions.length > 0)
        spacing: 8
        Rectangle {
          width: 16; height: 16; radius: 8
          anchors.verticalCenter: parent.verticalCenter
          visible: root.stamp !== ""
          color: root.stamp === "Denied" ? "#DC2626" : root.stamp.endsWith("allowed") || root.stamp === "Allowed"
              ? "#16A34A" : root.muted
          Image {
            anchors.centerIn: parent; width: 10; height: 10
            visible: root.stamp === "Allowed" || root.stamp === "Always allowed"
            source: Qt.resolvedUrl("../assets/check.svg")
            sourceSize: Qt.size(20, 20)
          }
          Text {
            anchors.centerIn: parent
            visible: root.stamp === "Denied"
            text: "✕"; color: "white"
            font.family: Shell.Theme.fontFamily; font.pixelSize: 9; font.weight: Font.Bold
          }
        }
        Text {
          anchors.verticalCenter: parent.verticalCenter
          text: root.stamp || "Sender gone — open the agent to answer."
          color: root.stamp ? root.ink : root.muted
          font.family: Shell.Theme.fontFamily; font.pixelSize: 12
          font.weight: root.stamp ? Font.DemiBold : Font.Normal
        }
      }

      // Suggested replies, one click each.
      Item { width: 1; height: 12; visible: chipRow.visible }
      Flow {
        id: chipRow
        width: parent.width; spacing: 8
        visible: root.canReply && root.suggestions.length > 0
        Repeater {
          model: root.suggestions
          delegate: Notifications.AgentButton {
            required property var modelData
            label: modelData
            pill: true
            ink: root.ink; surface: root.surface; line: root.line; hoverFill: root.hoverFill
            onClicked: root.send(modelData)
          }
        }
      }

      Item { width: 1; height: chipRow.visible ? 8 : 12; visible: reply.visible }
      Rectangle {
        id: reply
        width: parent.width; height: 36; radius: 10
        visible: root.canReply
        color: Qt.rgba(1, 1, 1, 0.6)
        border.width: field.activeFocus ? 2 : 1
        border.color: field.activeFocus ? root.focusRing : root.line

        MouseArea {
          anchors.fill: parent
          cursorShape: Qt.IBeamCursor
          onPressed: field.forceActiveFocus()
        }
        TextInput {
          id: field
          x: 12; width: parent.width - x - 40
          anchors.verticalCenter: parent.verticalCenter
          color: root.ink; selectionColor: "#CBA6F7"; selectedTextColor: root.ink
          font.family: Shell.Theme.fontFamily; font.pixelSize: 13
          clip: true; maximumLength: 500
          Keys.onReturnPressed: root.send(text)
          Keys.onEnterPressed: root.send(text)
          Keys.onEscapePressed: { text = ""; focus = false; }
          Text {
            anchors.verticalCenter: parent.verticalCenter
            text: root.notification.inlineReplyPlaceholder || ("Reply to " + root.identity.name + "…")
            visible: field.text === ""
            color: root.muted; font: field.font
          }
        }
        Rectangle {
          x: parent.width - 32; y: 4; width: 28; height: 28; radius: 8
          color: root.ink
          opacity: field.text.trim() ? 1 : 0.3
          Behavior on opacity { NumberAnimation { duration: 160 } }
          Image {
            anchors.centerIn: parent; width: 14; height: 14
            source: Qt.resolvedUrl("../assets/send.svg")
            sourceSize: Qt.size(28, 28)
          }
          MouseArea {
            anchors.fill: parent
            enabled: field.text.trim() !== ""
            cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: root.send(field.text)
          }
        }
      }
      Item { width: 1; height: 4; visible: hint.visible }
      Text {
        id: hint
        visible: reply.visible && field.activeFocus
        text: "Enter to send · Esc to clear"
        color: root.muted; font.family: Shell.Theme.fontFamily; font.pixelSize: 11
        lineHeight: 16; lineHeightMode: Text.FixedHeight
      }

      // Delivery status: sending → sent (green check), or failed (red "!" + Open).
      Item { width: 1; height: 12; visible: root.sent }
      Row {
        visible: root.sent
        spacing: 8
        Rectangle {
          width: 16; height: 16; radius: 8
          visible: root.delivery !== "sending"
          color: root.delivery === "failed" ? "#DC2626" : "#16A34A"
          anchors.verticalCenter: parent.verticalCenter
          Image {
            anchors.centerIn: parent; width: 10; height: 10
            visible: root.delivery === "sent"
            source: Qt.resolvedUrl("../assets/check.svg")
            sourceSize: Qt.size(20, 20)
          }
          Text {
            anchors.centerIn: parent
            visible: root.delivery === "failed"
            text: "!"; color: "white"
            font.family: Shell.Theme.fontFamily; font.pixelSize: 11; font.weight: Font.ExtraBold
          }
        }
        Text {
          anchors.verticalCenter: parent.verticalCenter
          text: root.delivery === "sending" ? "Sending…"
              : root.delivery === "failed" ? "Couldn’t deliver to " + root.identity.name
              : root.pickedUp ? "Sent · " + root.identity.name + " is on it"
              : "Sent to " + root.identity.name
          color: root.delivery === "failed" ? root.ink : root.muted
          font.family: Shell.Theme.fontFamily; font.pixelSize: 12
          font.weight: root.delivery === "failed" ? Font.DemiBold : Font.Normal
        }
      }
      Item { width: 1; height: 4; visible: failedHelp.visible }
      Text {
        id: failedHelp
        width: parent.width
        visible: root.delivery === "failed"
        text: "The session isn’t listening anymore. Open it to reply there."
        color: root.muted; font.family: Shell.Theme.fontFamily; font.pixelSize: 12
        wrapMode: Text.Wrap; lineHeight: 16; lineHeightMode: Text.FixedHeight
      }
      Item { width: 1; height: 12; visible: root.delivery === "failed" }
      Notifications.AgentButton {
        visible: root.delivery === "failed"
        label: "Open session"
        primary: true
        ink: root.ink; surface: root.surface; line: root.line; hoverFill: root.hoverFill
        onClicked: root.activated()
      }
    }

    // 28px round close, centred on the sender line (16 top pad + 10 half line = 26).
    Rectangle {
      id: closeButton
      x: parent.width - 12 - width; y: body.y - 4
      width: 28; height: 28; radius: 14
      color: closeArea.containsMouse || activeFocus ? root.hoverFill : "transparent"
      // Keyboard: Tab to it, then Enter, Space or Delete dismisses.
      activeFocusOnTab: true
      border.width: activeFocus ? 2 : 0
      border.color: root.focusRing
      Accessible.role: Accessible.Button
      Accessible.name: "Dismiss"
      Accessible.onPressAction: root.dismissed()
      Keys.onPressed: event => {
        if ([Qt.Key_Return, Qt.Key_Enter, Qt.Key_Space, Qt.Key_Delete].includes(event.key)) {
          root.dismissed();
          event.accepted = true;
        }
      }
      Repeater {
        model: [45, -45]
        Rectangle {
          required property var modelData
          anchors.centerIn: parent
          width: 11; height: 1.6; radius: 0.8
          rotation: modelData
          color: closeArea.containsMouse ? root.ink : root.muted
          antialiasing: true
        }
      }
      MouseArea {
        id: closeArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: root.dismissed()
      }
    }

    // Auto-dismiss: 2px bar along the bottom edge, drains over 8s, pauses while held.
    Rectangle {
      x: 16; y: parent.height - 2; height: 2; radius: 1
      width: (parent.width - 32) * root.remaining
      visible: !root.sticky && !root.sent
      color: root.ink; opacity: 0.35
    }
  }

  // Mascot, positioned relative to the card per layout.
  ClippingRectangle {
    id: mascot
    width: root.mascotSize; height: root.mascotSize
    transform: Translate { id: mascotShift }
    // Waiting breathes, slowly, so it reads as "still here" without asking twice.
    SequentialAnimation on scale {
      running: root.motion && root.kind === "needs" && root.visible
      loops: Animation.Infinite
      alwaysRunToEnd: true
      NumberAnimation { to: 1.04; duration: 1400; easing.type: Easing.InOutSine }
      NumberAnimation { to: 1; duration: 1400; easing.type: Easing.InOutSine }
    }
    x: root.layout === "corner" ? card.x - 16
       : root.layout === "floating" ? card.x + 20
       : root.layout === "sitting" ? card.x + (card.width - width) / 2
       : card.x + 16
    y: root.layout === "corner" ? card.y - 20
       : root.layout === "floating" ? card.y - 8 - height
       : root.layout === "sitting" ? card.y + 8 - height
       : card.y + 16
    radius: root.look.avatar ? width / 2 : 0
    color: root.look.avatar ? "#F4F1F5" : "transparent"
    IconImage {
      anchors.fill: parent
      source: root.iconSource
      asynchronous: true; smooth: true
    }
  }

  // Done: the mascot hops once. Failed: the card nudges once. Both skipped with reduced motion.
  SequentialAnimation {
    id: hop
    NumberAnimation { target: mascotShift; property: "y"; to: -8; duration: 160; easing.type: Easing.OutQuad }
    NumberAnimation { target: mascotShift; property: "y"; to: 0; duration: 260; easing.type: Easing.OutBack }
  }
  SequentialAnimation {
    id: nudge
    NumberAnimation { target: cardShift; property: "x"; to: -4; duration: 60 }
    NumberAnimation { target: cardShift; property: "x"; to: 4; duration: 90 }
    NumberAnimation { target: cardShift; property: "x"; to: 0; duration: 120; easing.type: Easing.OutBack }
  }

  NumberAnimation {
    id: countdown
    target: root; property: "remaining"
    from: 1; to: 0; duration: 8000
    running: !root.sticky && !root.sent && root.visible
    // Paused while held, and while you're away: it waits for you instead of expiring.
    paused: running && (root.held || Notifications.State.away)
    onFinished: if (root.remaining <= 0) root.expired()
  }

  // Others are waiting their turn: a Done card gets 3s instead of the full 8s, so a burst of
  // finishing agents doesn't take a minute to get through.
  property real shownAt: 0
  onVisibleChanged: if (visible) shownAt = Date.now()
  Timer {
    interval: 250; repeat: true
    running: root.visible && !root.sticky && !root.sent
    onTriggered: if (Notifications.State.queue.length && !root.held && !Notifications.State.away
                     && !root.stamp && Date.now() - root.shownAt >= 3000) root.expired()
  }

  // Proof it landed: after "Sent", watch the agent's terminal title for up to 4s. If the agent
  // starts working, say so; either way the toast then closes. Nothing is claimed it didn't see.
  property bool pickedUp: false
  Timer {
    id: pickup
    property string forId: ""
    property real since: 0
    interval: 250; repeat: true
    onTriggered: {
      if (forId !== root.shownId) { stop(); return; }
      const w = Shell.NiriMsg._windows[root.notification.windowId];
      const match = w ? Notifications.State.agentForTitle(w.title) : null;
      if (match && match.state === "working") root.pickedUp = true;
      if (root.pickedUp || Date.now() - since > 4000) {
        stop();
        sentClose.forId = forId;
        sentClose.start();
      }
    }
  }
  Timer {
    id: sentClose
    property string forId: ""
    interval: 1200
    onTriggered: if (forId === root.shownId) root.dismissed()
  }
}
