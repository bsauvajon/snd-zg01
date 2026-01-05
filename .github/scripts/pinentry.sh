#!/bin/sh
# Simple pinentry script for CI: read passphrase from a file and print it on demand
case "$1" in
  --version)
    echo "pinentry-tty 1.0"
    exit 0
    ;;
esac
while read line; do
  case "$line" in
    GETPIN)
      if [ -f "$HOME/.gnupg/passphrase" ]; then
        cat "$HOME/.gnupg/passphrase"
      else
        echo "ERROR: no passphrase" >&2
        exit 1
      fi
      ;;
    BYE)
      exit 0
      ;;
  esac
done
