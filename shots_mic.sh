#!/usr/bin/env bash
# Drives the hold-to-talk path with a spoken clip: COLOURING_AUTOHOLD holds the key,
# DATA_DIR/fake-mic.wav is the microphone. usage: shots_mic.sh DATA_DIR
set -u
here=$(cd "$(dirname "$0")" && pwd)
data=$1
rt=$(mktemp -d /tmp/agent-wl.XXXX); chmod 700 $rt
printf 'xwayland disable\noutput HEADLESS-1 resolution 1280x800\ndefault_border none\n' > $rt/config
E="env -i HOME=$HOME PATH=$PATH XDG_RUNTIME_DIR=$rt"
$E WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c $rt/config >$rt/sway.log 2>&1 &
sway_pid=$!; sleep 2
$E WAYLAND_DISPLAY=wayland-1 WGPU_BACKEND=vulkan VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json COLOURING_PAGES_HOME=$data COLOURING_AUTOHOLD=${HOLD:-3} $here/target/debug/colouring-pages >$rt/app.log 2>&1 &
app_pid=$!; sleep 4
$E WAYLAND_DISPLAY=wayland-1 grim $here/shots/mic-listening.png
sleep ${READ_WAIT:-20}
$E WAYLAND_DISPLAY=wayland-1 grim $here/shots/mic-after.png
kill $app_pid $sway_pid 2>/dev/null; tail -5 $rt/app.log; rm -rf $rt
