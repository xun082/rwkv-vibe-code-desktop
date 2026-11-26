#!/usr/bin/env bash
set -euo pipefail

work=$(mktemp -d "$RUNNER_TEMP/rwkv-upgrade.XXXXXX")
case "$1" in
  deb)
    fixture=upgrade-fixture/fixture.deb
    candidate="release/RWKV-Vibe-Code_${VERSION}_amd64.deb"
    package=$(dpkg-deb -f "$fixture" Package)
    test "$(dpkg-deb -f "$fixture" Version)" = 0.2.5
    test "$(dpkg-deb -f "$candidate" Package)" = "$package"
    test "$(dpkg-deb -f "$candidate" Version)" = "$VERSION"
    sudo -n apt-get install -y --no-install-recommends "$PWD/$fixture"
    dpkg-deb -x "$candidate" "$work/candidate"
    desktop=/usr/bin/local-page-studio
    expected="$work/candidate$desktop"

    # Hosted runners have passwordless sudo and no interactive authentication agent.
    mkdir "$work/bin"
    for command in pkexec zenity kdialog; do
      printf '#!/bin/sh\nexit 1\n' > "$work/bin/$command"
    done
    printf '#!/bin/sh\nexec /usr/bin/sudo -n "$@"\n' > "$work/bin/sudo"
    chmod 755 "$work/bin/"*
    export PATH="$work/bin:$PATH"
    ;;
  appimage)
    desktop="$work/fixture.AppImage"
    expected="release/RWKV-Vibe-Code_${VERSION}_amd64.AppImage"
    install -m 755 upgrade-fixture/fixture.AppImage "$desktop"
    export APPIMAGE_EXTRACT_AND_RUN=1
    ;;
  *) exit 2 ;;
esac
test -x "$desktop"
test -f "$expected"
if cmp -s "$desktop" "$expected"; then
  echo 'Fixture already matches the release binary.' >&2
  exit 1
fi

export XDG_DATA_HOME="$work/data"
identifier=$(jq -er .identifier src-tauri/tauri.conf.json)
mkdir -p "$XDG_DATA_HOME/$identifier"
sentinel="$XDG_DATA_HOME/$identifier/upgrade-smoke.txt"
printf '%s' preserve-data > "$sentinel"

WEBKIT_DISABLE_DMABUF_RENDERER=1 "$desktop" > "$work/desktop.log" 2>&1 &
app=$!
trap 'result=$?; kill "$app" 2>/dev/null || true; if [[ "$result" != 0 ]]; then cat "$work/desktop.log"; fi' EXIT
for ((attempt = 0; attempt < 120; attempt++)); do
  if cmp -s "$desktop" "$expected"; then
    if [[ "$1" == deb ]] && [[ "$(dpkg-query -W -f='${Version} ${db:Status-Status}' "$package")" != "$VERSION installed" ]]; then
      sleep 2
      continue
    fi
    test "$(cat "$sentinel")" = preserve-data
    echo "$1 fixture upgraded to the exact $VERSION release binary and preserved app data."
    exit 0
  fi
  sleep 2
done
echo "$1 fixture did not install version $VERSION." >&2
exit 1
