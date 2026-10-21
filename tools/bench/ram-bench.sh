#!/usr/bin/env bash
# Reproducible RAM benchmark for cmux-linux.
#
# Launches cmux-app with a clean profile, drives it through the `cmux` CLI (socket API) and
# samples memory of the cmux-app process (shells excluded; reported separately).
#
# Scenarios:
#   1. idle      - 1 workspace / 1 pane, 15 s after launch
#   2. loaded    - 4 workspaces x 3 panes, each pane printed 20,000 lines of 150 chars
#   3. switching - after 60 workspace switches
#
# Usage: tools/bench/ram-bench.sh <path/to/cmux-app> <path/to/cmux> [label]
set -euo pipefail

app=${1:?usage: ram-bench.sh <cmux-app> <cmux> [label]}
cli=${2:?usage: ram-bench.sh <cmux-app> <cmux> [label]}
label=${3:-run}

pkill -x cmux-app 2>/dev/null || true
sleep 1
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/cmux"   # no restored session

"$app" >/tmp/cmux-bench-"$label".log 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true' EXIT

for _ in $(seq 1 60); do "$cli" ping >/dev/null 2>&1 && break; sleep 0.25; done

sample() {
    local scenario=$1
    sleep 5
    local rss priv threads shells
    rss=$(awk '/^VmRSS/ {print $2}' "/proc/$pid/status")
    priv=$(awk '/^Private_(Clean|Dirty):/ {s += $2} END {print s}' "/proc/$pid/smaps_rollup")
    threads=$(awk '/^Threads/ {print $2}' "/proc/$pid/status")
    shells=$(pgrep -c -P "$pid" || true)
    printf '%-10s %-44s rss=%6.1f MB  private=%6.1f MB  threads=%3d  children=%d\n' \
        "$label" "$scenario" "$(echo "$rss/1024" | bc -l)" "$(echo "$priv/1024" | bc -l)" "$threads" "$shells"
}

sleep 10
sample "1-idle (1 workspace, 1 pane)"

flood='yes "$(printf "A%.0s" $(seq 150))" | head -n 20000'
for w in 1 2 3 4; do
    if [ "$w" -gt 1 ]; then "$cli" new-workspace >/dev/null; sleep 1; fi
    "$cli" split --direction horizontal >/dev/null 2>&1 || "$cli" split >/dev/null; sleep 1
    "$cli" split --direction vertical >/dev/null 2>&1 || "$cli" split >/dev/null; sleep 1
done
sleep 2
# Flood every pane (list-surfaces returns panes of all workspaces).
ids() { python3 -c 'import json,sys; d=json.load(sys.stdin); print(chr(10).join(x.get("uuid") or str(x.get("id")) for x in d[sys.argv[1]]))' "$1"; }
mapfile -t workspaces < <("$cli" --json list-workspaces | ids workspaces)
mapfile -t panes < <("$cli" --json list-surfaces | ids surfaces)
for s in "${panes[@]}"; do "$cli" send-text --id "$s" "$flood"$'
' >/dev/null 2>&1 || true; done
echo "workspaces=${#workspaces[@]} panes=${#panes[@]}"
sleep 20
sample "2-loaded (4 workspaces x 3 panes, 20k-line floods)"

for i in $(seq 0 59); do
    "$cli" select-workspace "${workspaces[$((i % ${#workspaces[@]}))]}" >/dev/null
    sleep 0.15
done
sample "3-after 60 workspace switches"
