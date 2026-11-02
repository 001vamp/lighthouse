import QtQuick
import Quickshell
import Quickshell.Wayland
import "." as Notifications
import ".." as Shell

// Collapsed-sidebar arrival surface. Its left edge tracks the animated
// sidebar edge, so it never temporarily overlaps and then jumps with the
// compositor's work-area change.
PanelWindow {
  id: root

  required property var bar

  readonly property string outputName: bar && bar.screen ? bar.screen.name : ""
  readonly property bool outputMatches: Notifications.State.transientOutputName === ""
      || Notifications.State.transientOutputName === outputName
  // From the displayed notification, not the current one: during the exit animation the
  // current one is already gone, and the leaving toast must keep its own look and size.
  readonly property bool agentToast: Notifications.State.agentInfo(displayedNotification) !== null
  readonly property bool requested: outputMatches
      && Notifications.State.transientVisible
      && (bar.collapseProgress > 0.42 || agentToast)
  property real reveal: requested ? 1 : 0
  property var displayedNotification: ({})

  screen: bar.screen
  visible: requested || reveal > 0.001
  color: "transparent"
  implicitWidth: agentToast ? agentCard.implicitWidth : Notifications.Style.interactiveWidth
  implicitHeight: agentToast ? agentCard.implicitHeight : toastCard.implicitHeight

  anchors {
    left: true
    top: true
  }
  // The agent toast carries its own shadow padding and mascot overhang, so the window
  // starts that much further left/up and the card itself still lines up with other toasts.
  margins {
    left: Math.round(Shell.Theme.sidebarOuterMargin + bar.cardWidth
                     + Notifications.Style.desktopMargin)
        - (agentToast ? agentCard.pad + agentCard.leftInset : 0)
    top: agentToast ? 0 : Notifications.Style.desktopMargin
  }

  WlrLayershell.layer: WlrLayer.Overlay
  WlrLayershell.namespace: "pond-notification-toast-"
      + (outputName || "unknown")
  WlrLayershell.exclusionMode: ExclusionMode.Ignore
  // Take the keyboard only when clicked, and only when there's something to type or press.
  WlrLayershell.keyboardFocus: agentToast && (agentCard.canReply || agentCard.actions.length > 0)
      ? WlrKeyboardFocus.OnDemand : WlrKeyboardFocus.None

  Behavior on reveal { Shell.Motion { duration: Notifications.Style.panelDuration } }

  Component.onCompleted: {
    if (Notifications.State.currentTransient)
      displayedNotification = Notifications.State.currentTransient;
  }

  Connections {
    target: Notifications.State
    function onCurrentTransientChanged() {
      if (Notifications.State.currentTransient)
        root.displayedNotification = Notifications.State.currentTransient;
    }
  }

  Notifications.Card {
    id: toastCard
    x: (1 - root.reveal) * -4
    y: 0
    width: Notifications.Style.interactiveWidth
    height: implicitHeight
    opacity: root.agentToast ? 0 : root.reveal
    visible: opacity > 0.001
    notification: root.displayedNotification
    onActivated: Notifications.State.activate(root.displayedNotification)
    onDismissed: Notifications.State.remove(root.displayedNotification)
  }
  Notifications.AgentToast {
    id: agentCard
    // Library entrance: drop in 12px while fading.
    y: (1 - root.reveal) * -12
    width: implicitWidth
    height: implicitHeight
    opacity: root.agentToast ? root.reveal : 0
    visible: opacity > 0.001
    notification: root.displayedNotification
    onActivated: Notifications.State.activate(root.displayedNotification)
    onDismissed: Notifications.State.remove(root.displayedNotification)
    onExpired: Notifications.State.hideTransient(root.displayedNotification)
    onReplied: text => Notifications.State.reply(root.displayedNotification, text)
    onActionInvoked: identifier => Notifications.State.invokeAction(root.displayedNotification, identifier)
  }
}
