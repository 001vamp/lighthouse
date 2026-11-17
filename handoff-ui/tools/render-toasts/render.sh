#!/bin/sh
# Render AgentToast.qml offscreen with stub Pond parts, to a PNG. No desktop needed.
#   handoff-ui/tools/render-toasts/render.sh [out.png]
# Edit the sample notifications in shell.qml. Fonts: install Onest for a faithful render.
set -e
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
work=$(mktemp -d)
cp -r "$here"/. "$work"/
cp "$repo"/toasty/shell/files/notifications/AgentToast.qml "$repo"/toasty/shell/files/notifications/AgentButton.qml "$work"/notifications/
mkdir -p "$work"/assets/agents
cp "$repo"/toasty/shell/files/assets/agents/*.png "$work"/assets/agents/
cp "$repo"/toasty/shell/files/assets/*.svg "$work"/assets/ 2>/dev/null || true
# Pond's own pieces the toast imports.
pond=/usr/share/pond-interface/shell
cp "$pond"/Shadow.qml "$work"/
[ -e "$work"/assets/check.svg ] || cp "$pond"/assets/check.svg "$work"/assets/
# Offscreen grabs drop IconImage/ClippingRectangle, so the mascot uses plain types here only.
sed -i 's/IconImage {/Image {/; s/ClippingRectangle {/Rectangle {/' "$work"/notifications/AgentToast.qml
OUT="${1:-$PWD/toasts.png}" QT_QPA_PLATFORM=offscreen timeout 20 qs -p "$work" >/dev/null 2>&1 || true
rm -rf "$work"
echo "${1:-$PWD/toasts.png}"
