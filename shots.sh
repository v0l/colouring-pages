#!/usr/bin/env bash
# usage: shots.sh NAME WIDTH HEIGHT [ENV=VAL ...]   writes shots/NAME.png
set -u
name=$1; w=$2; h=$3; shift 3
bin=$(dirname "$0")/target/debug/colouring-pages
rt=$(mktemp -d /tmp/agent-wl.XXXX); chmod 700 $rt
printf 'xwayland disable\noutput HEADLESS-1 resolution %sx%s\ndefault_border none\n' "$w" "$h" > $rt/config
E="env -i HOME=$HOME PATH=$PATH XDG_RUNTIME_DIR=$rt"
$E WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c $rt/config >$rt/sway.log 2>&1 &
sway_pid=$!; sleep 2
$E WAYLAND_DISPLAY=wayland-1 WGPU_BACKEND=vulkan VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json "$@" $bin >$rt/app.log 2>&1 &
app_pid=$!
sleep "${SHOT_WAIT:-6}"
mkdir -p "$(dirname "$0")/shots"
$E WAYLAND_DISPLAY=wayland-1 grim "$(dirname "$0")/shots/$name.png"
kill $app_pid $sway_pid 2>/dev/null; tail -3 $rt/app.log; rm -rf $rt
