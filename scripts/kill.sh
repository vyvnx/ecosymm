#!/usr/bin/env bash
# kill anything started from this repo - the ws server, vite, a cargo run.
set -uo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

# match by repo path (vite, cargo) or by binary name (the server runs from a
# relative ./target path, so its cmdline never mentions $root)
pids=$(pgrep -f "$root|ecosym-server|ecosym-cli" | grep -vxE "$$|$PPID")
if [[ -z $pids ]]; then
  echo "nothing running"
  exit 0
fi

ps -o pid=,args= -p $pids | cut -c1-100
kill $pids
