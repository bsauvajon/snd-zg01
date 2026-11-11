#!/usr/bin/env bash
set -euo pipefail
command -v amixer >/dev/null || { echo "amixer not installed"; exit 0; }
grep -q '\[zg01' /proc/asound/cards 2>/dev/null || { echo "no zg01 card"; exit 0; }

controls=$(amixer -c zg01 controls)
grep -q "Meter Game Mix" <<< "$controls"
grep -q "Meter HDMI In" <<< "$controls"

# cget prints the control info (including values=N) before it reads the
# value.  While the meter session is blocked, the getter returns -ENODATA
# and amixer exits nonzero after that info line; the registration/count
# contract is still valid, so ignore the read status and parse the info.
read_control() {
    amixer -c zg01 cget "name=$1" 2>&1 || true
}

mic_in=$(read_control 'Meter Mic In')
grep -q 'values=1' <<< "$mic_in"

pc_in=$(read_control 'Meter PC In')
grep -q 'values=8' <<< "$pc_in"

stream_mix=$(read_control 'Meter Stream Mix')
grep -q 'values=8' <<< "$stream_mix"

echo "zg01 meter controls contract passed"
