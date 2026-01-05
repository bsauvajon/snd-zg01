#!/usr/bin/env bash
set -euo pipefail

ROOT_CMD="sudo"
OUTDIR="/tmp/zg01_capture_test"
mkdir -p "$OUTDIR"

usage() {
  echo "Usage: $0 [rates...]" >&2
  echo "Example: $0 48000 16000" >&2
  exit 1
}

if [ $# -lt 1 ]; then
  usage
fi

for rate in "$@"; do
  outfile="$OUTDIR/capture_${rate}.wav"
  echo "\n=== Test capture at ${rate} Hz -> ${outfile} ==="
  echo "Reloading modules..."
  ./scripts/reload_modules.sh
  sleep 0.5
  echo "Starting capture for 5s at ${rate}..."
  arecord -f S32_LE -r "$rate" -c 2 -d 5 "$outfile" || true
  echo "Saved: $outfile"
  echo "dmesg (last 60 lines):"
  ${ROOT_CMD} dmesg -T | tail -60
  echo "hexdump (first 256 bytes after 44-byte WAV header):"
  xxd -g 4 -c 16 -s 44 -l 256 "$outfile" || true
done

echo "All captures saved under $OUTDIR"
