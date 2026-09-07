#!/usr/bin/env bash

set -euo pipefail

cycles=100

usage() {
  printf 'Usage: %s [--cycles N]\n' "${0##*/}"
}

while (($#)); do
  case "$1" in
    --cycles)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      cycles=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

[[ $cycles =~ ^[1-9][0-9]*$ ]] || {
  printf 'Cycle count must be a positive integer\n' >&2
  exit 2
}

for command in jq pactl pw-dump pw-play pw-record timeout wpctl; do
  command -v "$command" >/dev/null || {
    printf 'Missing required command: %s\n' "$command" >&2
    exit 1
  }
done

bounded() {
  timeout --signal=TERM --kill-after=1s 2s "$@"
}

bounded wpctl status >/dev/null
bounded pactl info >/dev/null

nodes=$(bounded pw-dump)
log_dir=$(mktemp -d)
trap 'rm -rf "$log_dir"' EXIT

find_node() {
  local media_class=$1
  local label=$2

  jq -r --arg media_class "$media_class" --arg label "$label" '
    .[]
    | select(.type == "PipeWire:Interface:Node")
    | .info.props
    | select(."media.class" == $media_class)
    | select([
        (."node.description" // ""),
        (."node.nick" // ""),
        (."alsa.card_name" // "")
      ] | join(" ") | test($label; "i"))
    | ."node.name"
  ' <<<"$nodes" | head -n 1
}

game_node=$(find_node Audio/Sink 'ZG01 Game')
voice_out_node=$(find_node Audio/Sink 'ZG01 Voice Out')
voice_in_node=$(find_node Audio/Source 'ZG01 Voice In')

for node_spec in \
  "Game sink:$game_node" \
  "Voice Out sink:$voice_out_node" \
  "Voice In source:$voice_in_node"
do
  [[ ${node_spec#*:} ]] || {
    printf 'Missing PipeWire node: %s\n' "${node_spec%%:*}" >&2
    exit 1
  }
done

start_time=$(date --iso-8601=seconds)

run_cycle() {
  local game_pid voice_out_pid voice_in_pid
  local game_rc voice_out_rc voice_in_rc
  local game_log="$log_dir/game.log"
  local voice_out_log="$log_dir/voice-out.log"
  local voice_in_log="$log_dir/voice-in.log"

  bounded pw-play --verbose --raw --rate 48000 --channels 2 --format s32 \
    --sample-count 4800 --target "$game_node" /dev/zero \
    >"$game_log" 2>&1 &
  game_pid=$!

  bounded pw-play --verbose --raw --rate 48000 --channels 2 --format s32 \
    --sample-count 4800 --target "$voice_out_node" /dev/zero \
    >"$voice_out_log" 2>&1 &
  voice_out_pid=$!

  bounded pw-record --verbose --raw --rate 48000 --channels 2 --format s32 \
    --sample-count 4800 --target "$voice_in_node" /dev/null \
    >"$voice_in_log" 2>&1 &
  voice_in_pid=$!

  set +e
  wait "$game_pid"
  game_rc=$?
  wait "$voice_out_pid"
  voice_out_rc=$?
  wait "$voice_in_pid"
  voice_in_rc=$?
  set -e

  # pw-cat 1.6.8 exits 1 after a completed --sample-count stream. Accept that
  # code only when the client proves it reached PipeWire's streaming state.
  if ! { ((game_rc == 0)) || { ((game_rc == 1)) && grep -q 'paused -> streaming' "$game_log"; }; } ||
     ! { ((voice_out_rc == 0)) || { ((voice_out_rc == 1)) && grep -q 'paused -> streaming' "$voice_out_log"; }; } ||
     ! { ((voice_in_rc == 0)) || { ((voice_in_rc == 1)) && grep -q 'paused -> streaming' "$voice_in_log"; }; }
  then
    printf 'Stream cycle failed: game=%d voice_out=%d voice_in=%d\n' \
      "$game_rc" "$voice_out_rc" "$voice_in_rc" >&2
    tail -n 10 "$game_log" "$voice_out_log" "$voice_in_log" >&2
    return 1
  fi
}

for ((cycle = 1; cycle <= cycles; cycle++)); do
  run_cycle

  if ((cycle % 10 == 0)); then
    bounded wpctl status >/dev/null
    bounded pactl info >/dev/null
    printf 'ZG01 lifecycle cycles: %d/%d\n' "$cycle" "$cycles"
  fi
done

if ps -eLo stat=,comm=,wchan= | awk '
  $1 ~ /^D/ && ($2 == "pipewire" || $2 ~ /^data-loop/) { found = 1 }
  END { exit !found }
'; then
  printf 'PipeWire has a thread in uninterruptible sleep\n' >&2
  exit 1
fi

kernel_log=$(journalctl -k --since "$start_time" --no-pager)
if grep -Eiq \
  'blocked for more than|BUG:|WARNING: CPU|Oops:|general protection fault|zg01.*(failed|error)|usb.*(descriptor read|reset).*0499:1513' \
  <<<"$kernel_log"
then
  printf 'Kernel log contains a ZG01 lifecycle failure\n' >&2
  grep -Ei \
    'blocked for more than|BUG:|WARNING: CPU|Oops:|general protection fault|zg01.*(failed|error)|usb.*(descriptor read|reset).*0499:1513' \
    <<<"$kernel_log" >&2
  exit 1
fi

default_source=$(bounded pactl get-default-source)
[[ $default_source == *Yamaha_Corporation_Yamaha_ZG01* ]] || {
  printf 'Unexpected default source: %s\n' "$default_source" >&2
  exit 1
}

[[ $(bounded pactl get-source-mute "$default_source") == 'Mute: no' ]] || {
  printf 'Default ZG01 source is muted\n' >&2
  exit 1
}

if bounded pactl get-source-volume "$default_source" | grep -Eq '/[[:space:]]+0%[[:space:]]+/'
then
  printf 'Default ZG01 source volume is 0%%\n' >&2
  exit 1
fi

printf 'ZG01 lifecycle stress passed: %d cycles\n' "$cycles"
