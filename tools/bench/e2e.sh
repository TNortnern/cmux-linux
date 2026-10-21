#!/usr/bin/env bash
# End-to-end checks for cmux-linux, driven through the `cmux` CLI (socket API).
# Launches cmux-app with a clean profile several times; prints a PASS/FAIL table; exit 1 on any FAIL.
#
# Usage: tools/bench/e2e.sh [target/release]
set -uo pipefail

bin=${1:-target/release}
app="$bin/cmux-app"; cli="$bin/cmux"
data="${XDG_DATA_HOME:-$HOME/.local/share}/cmux"
conf="${XDG_CONFIG_HOME:-$HOME/.config}/cmux"
root=$(mktemp -d "/tmp/cmux e2e.XXXX")          # space on purpose
for d in A B C D E; do mkdir -p "$root/dir $d"; done
pass=0; fail=0
check() { # name, condition-result(0/1), detail
    if [ "$2" -eq 0 ]; then pass=$((pass+1)); printf 'PASS  %-58s %s\n' "$1" "$3"
    else fail=$((fail+1)); printf 'FAIL  %-58s %s\n' "$1" "$3"; fi
}
j() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
pid=""
start() { "$app" >"/tmp/cmux-e2e.log" 2>&1 & pid=$!
          for _ in $(seq 60); do "$cli" ping >/dev/null 2>&1 && break; sleep 0.25; done; sleep 2; }
stop()  { kill -TERM "$pid" 2>/dev/null; for _ in $(seq 40); do kill -0 "$pid" 2>/dev/null || break; sleep 0.25; done; }
run()   { "$cli" send-text "$1"$'\r' >/dev/null; sleep 1.5; }      # \r = Enter
cwd()   { "$cli" --json pane-cwd | j 'd["cwd"]'; }
tabs()  { "$cli" --json list-tabs; }
shells_of() { pgrep -P "$1" | xargs -r -I{} sh -c 'echo {}; pgrep -P {}' | wc -l; }

pkill -x cmux-app; sleep 1; rm -rf "$data"; mkdir -p "$conf"; rm -f "$conf/config.toml"

# --- 1. folders follow the focused pane ----------------------------------------------------
start
run "cd '$root/dir A'"
check "focused pane reports its folder" "$([ "$(cwd)" = "$root/dir A" ]; echo $?)" "$(cwd)"
"$cli" split --direction horizontal >/dev/null; sleep 2
check "split opens in the current folder" "$([ "$(cwd)" = "$root/dir A" ]; echo $?)" "$(cwd)"
run "cd '$root/dir B'"
"$cli" new-tab >/dev/null; sleep 2
check "new tab opens in the current folder" "$([ "$(cwd)" = "$root/dir B" ]; echo $?)" "$(cwd)"
run "cd '$root/dir C'"

# --- 2. tab operations ---------------------------------------------------------------------------
n=$(tabs | j 'len(d["tabs"])')
check "two tabs after new-tab" "$([ "$n" = 2 ]; echo $?)" "tabs=$n"
"$cli" select-tab 0 >/dev/null; sleep 1
check "select-tab 0 selects the first tab" "$([ "$(tabs | j '[t["selected"] for t in d["tabs"]].index(True)')" = 0 ]; echo $?)" ""
"$cli" next-tab >/dev/null; sleep 1
check "next-tab moves to tab 1" "$([ "$(tabs | j '[t["selected"] for t in d["tabs"]].index(True)')" = 1 ]; echo $?)" ""
"$cli" rename-tab "logs" >/dev/null; sleep 0.5
check "rename-tab renames the active tab" "$([ "$(tabs | j 'd["tabs"][1]["name"]')" = logs ]; echo $?)" ""
"$cli" move-tab 1 0 >/dev/null; sleep 0.5
check "move-tab reorders" "$([ "$(tabs | j 'd["tabs"][0]["name"]')" = logs ]; echo $?)" "$(tabs | j '[t["name"] for t in d["tabs"]]')"
"$cli" new-workspace >/dev/null; sleep 2
c0=$(cwd)
check "new workspace starts in the current folder" "$([ "$c0" = "$root/dir C" ]; echo $?)" "$c0"
run "cd '$root/dir D'"
"$cli" select-workspace "$("$cli" --json list-workspaces | j 'd["workspaces"][0]["id"]')" >/dev/null; sleep 1

# --- 3. restore after a normal quit -----------------------------------------------------------
shells_before=$(shells_of "$pid"); oldpid=$pid
stop
sleep 1
check "quit ends every shell" "$([ "$(pgrep -P "$oldpid" | wc -l)" = 0 ] && ! kill -0 "$oldpid" 2>/dev/null; echo $?)" "had $shells_before shell processes"
start
nws=$("$cli" --json list-workspaces | j 'len(d["workspaces"])')
check "restore: 2 workspaces" "$([ "$nws" = 2 ]; echo $?)" "workspaces=$nws"
names=$(tabs | j '",".join(t["name"] for t in d["tabs"])')
check "restore: tab names and order" "$([ "$names" = "logs,Tab 1" ]; echo $?)" "$names"
"$cli" select-tab 1 >/dev/null; sleep 2
check "restore: split kept (2 panes in Tab 1)" "$([ "$(tabs | j 'd["tabs"][1]["panes"]')" = 2 ]; echo $?)" ""
c1=$(cwd)
# Tab 1: left pane stayed in dir A, the focused split pane cd'd to dir B.
check "restore: Tab 1 focused pane folder" "$([ "$c1" = "$root/dir B" ]; echo $?)" "$c1"
"$cli" focus-pane >/dev/null      # no id = next pane (the other one)
sleep 1; c1b=$(cwd)
check "restore: Tab 1 other pane folder" "$([ "$c1b" = "$root/dir A" ]; echo $?)" "$c1b"
"$cli" select-tab 0 >/dev/null; sleep 2
c2=$(cwd)
check "restore: logs tab folder" "$([ "$c2" = "$root/dir C" ]; echo $?)" "$c2"
"$cli" select-workspace "$("$cli" --json list-workspaces | j 'd["workspaces"][1]["id"]')" >/dev/null; sleep 2
c3=$(cwd)
check "restore: second workspace folder" "$([ "$c3" = "$root/dir D" ]; echo $?)" "$c3"

# --- 4. crash safety: folder changed right before a hard kill ---------------------------------
run "cd '$root/dir E'"
sleep 2                                     # > 500 ms save debounce
kill -9 "$pid"; sleep 1
start
"$cli" select-workspace "$("$cli" --json list-workspaces | j 'd["workspaces"][1]["id"]')" >/dev/null; sleep 2
c4=$(cwd)
check "crash: last folder survives kill -9" "$([ "$c4" = "$root/dir E" ]; echo $?)" "$c4"
stop

# --- 5. opt-out --------------------------------------------------------------------------------
printf '[session]\nrestore = false\n' >"$conf/config.toml"
start
check "opt-out: starts fresh (1 workspace, 1 tab)" "$([ "$("$cli" --json list-workspaces | j 'len(d["workspaces"])')" = 1 ] && [ "$(tabs | j 'len(d["tabs"])')" = 1 ]; echo $?)" ""
stop
rm -f "$conf/config.toml"

echo "---- $pass passed, $fail failed"
for _ in $(seq 20); do rm -rf "$root" 2>/dev/null && break; sleep 0.5; done
[ "$fail" -eq 0 ]
