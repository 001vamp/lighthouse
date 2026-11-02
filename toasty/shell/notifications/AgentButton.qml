import QtQuick
import ".." as Shell

// 28px toast button from the Pond toast library.
// primary: filled with the toast's text colour. pill: suggested reply.
Rectangle {
  id: root
  property string label: ""
  property bool primary: false
  property bool pill: false
  property color ink: "#211E24"
  property color surface: "#E4DFE6"
  property color line: Qt.rgba(0, 0, 0, 0.14)
  property color hoverFill: Qt.rgba(0, 0, 0, 0.07)
  signal clicked

  activeFocusOnTab: true
  Accessible.role: Accessible.Button
  Accessible.name: label
  Accessible.onPressAction: root.clicked()
  Keys.onReturnPressed: event => { root.clicked(); event.accepted = true; }
  Keys.onEnterPressed: event => { root.clicked(); event.accepted = true; }
  Keys.onSpacePressed: event => { root.clicked(); event.accepted = true; }

  Rectangle {
    anchors.fill: parent
    anchors.margins: -3
    radius: root.radius + 3
    color: "transparent"
    border.width: 2
    border.color: "#CBA6F7"
    visible: root.activeFocus
  }

  implicitWidth: text.implicitWidth + 24
  implicitHeight: 28
  radius: pill ? 14 : 8
  color: primary ? ink : pointer.containsMouse ? hoverFill : "transparent"
  border.width: primary ? 0 : 1
  border.color: line
  opacity: primary && pointer.containsMouse ? 0.88 : 1
  antialiasing: true

  Behavior on color { ColorAnimation { duration: Shell.Theme.sidebarHoverDuration } }

  Text {
    id: text
    anchors.centerIn: parent
    text: root.label
    color: root.primary ? root.surface : root.ink
    font.family: Shell.Theme.fontFamily
    font.pixelSize: 12; font.weight: Font.DemiBold
  }
  MouseArea {
    id: pointer
    anchors.fill: parent
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    onClicked: root.clicked()
  }
}
