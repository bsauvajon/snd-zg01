#!/usr/bin/env bash
# Contract test for the MBDF container reader in tools/zgpreset.py.
#
# The Python test builds synthetic MBDF files in memory; no Yamaha or user
# preset is read or committed.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
exec python3 "$repo_root/tests/test-zgpreset.py"
