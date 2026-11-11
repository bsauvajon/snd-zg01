#!/usr/bin/env bash
set -euo pipefail
command -v amixer >/dev/null || { echo "amixer not installed"; exit 0; }
grep -q '\[zg01' /proc/asound/cards 2>/dev/null || { echo "no zg01 card"; exit 0; }
amixer -c zg01 controls | grep -q "Meter Game Mix"
amixer -c zg01 controls | grep -q "Meter HDMI In"
amixer -c zg01 cget 'name=Meter Mic In' | grep -q 'values=1'
amixer -c zg01 cget 'name=Meter PC In' | grep -q 'values=8'
amixer -c zg01 cget 'name=Meter Stream Mix' | grep -q 'values=8'
echo "zg01 meter controls contract passed"
