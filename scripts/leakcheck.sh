#!/bin/sh
# Memory-leak check with Apple's `leaks`.
#
# 1. CLI: `leaks --atExit` must report exactly 0 leaks.
# 2. App: `leaks` is run against the live app at 3 s and 13 s, with the
#    countdown and user-activity timers running, and against a minimal
#    Objective-C status-item app started at the same time. The app must not
#    leak more than the baseline (AppKit itself leaks a fixed set of blocks in
#    every GUI process) and must not grow between the two samples.
#
# Binaries are copied and re-signed with the get-task-allow entitlement so
# `leaks` can read all of their memory ("not debuggable" otherwise).
set -eu
cd "$(dirname "$0")/.."

WORK=$(mktemp -d)
DOMAIN=bluemeth-app-leakcheck   # NSUserDefaults domain = executable name
cleanup() {
    { [ -n "${APP:-}" ] && kill "$APP" && wait "$APP"; } 2>/dev/null || true
    { [ -n "${BASE:-}" ] && kill "$BASE" && wait "$BASE"; } 2>/dev/null || true
    defaults delete "$DOMAIN" >/dev/null 2>&1 || true
    rm -rf "$WORK"
}
trap cleanup EXIT

cargo build -p bluemeth-cli -p bluemeth-app
cp target/debug/bluemeth "$WORK/bluemeth"
cp target/debug/bluemeth-app "$WORK/$DOMAIN"
clang -fobjc-arc -framework Cocoa -o "$WORK/baseline" scripts/leak-baseline.m
for bin in bluemeth "$DOMAIN" baseline; do
    codesign -s - -f --entitlements scripts/debug.entitlements "$WORK/$bin" 2>/dev/null
done

export MallocStackLogging=1
count() { sed -n 's/.*: \([0-9][0-9]*\) leaks\{0,1\} for \([0-9][0-9]*\) total leaked bytes.*/\1/p' "$1" | head -1; }
bytes() { sed -n 's/.*: \([0-9][0-9]*\) leaks\{0,1\} for \([0-9][0-9]*\) total leaked bytes.*/\2/p' "$1" | head -1; }
status=0

echo "== CLI: hold -dimsu for 1 s, then exit"
leaks --atExit -- "$WORK/bluemeth" -dimsu -t 1 >"$WORK/cli-hold.txt" 2>&1 || true
echo "   $(count "$WORK/cli-hold.txt") leaks, $(bytes "$WORK/cli-hold.txt") bytes"
[ "$(count "$WORK/cli-hold.txt")" = 0 ] || status=1

echo "== CLI: run a utility"
leaks --atExit -- "$WORK/bluemeth" -i /usr/bin/true >"$WORK/cli-util.txt" 2>&1 || true
echo "   $(count "$WORK/cli-util.txt") leaks, $(bytes "$WORK/cli-util.txt") bytes"
[ "$(count "$WORK/cli-util.txt")" = 0 ] || status=1

echo "== App vs. baseline AppKit app (3 s and 13 s after launch)"
for key in preventDisplaySleep preventIdleSleep preventSystemSleep declareUserActive; do
    defaults write "$DOMAIN" "$key" -bool true
done
defaults write "$DOMAIN" timeoutSeconds -int 300   # 1 Hz countdown timer runs
"$WORK/baseline" 2>/dev/null & BASE=$!
"$WORK/$DOMAIN" 2>/dev/null & APP=$!
sleep 3
leaks "$APP" >"$WORK/app-3.txt" 2>&1 || true
leaks "$BASE" >"$WORK/base-3.txt" 2>&1 || true
sleep 10
leaks "$APP" >"$WORK/app-13.txt" 2>&1 || true
leaks "$BASE" >"$WORK/base-13.txt" 2>&1 || true
app3=$(count "$WORK/app-3.txt"); app13=$(count "$WORK/app-13.txt")
base3=$(count "$WORK/base-3.txt"); base13=$(count "$WORK/base-13.txt")
printf '   %-22s %6s leaks %7s bytes | %6s leaks %7s bytes\n' "" "t=3s" "" "t=13s" ""
printf '   %-22s %6s       %7s       | %6s       %7s\n' "Bluemeth app" "$app3" "$(bytes "$WORK/app-3.txt")" "$app13" "$(bytes "$WORK/app-13.txt")"
printf '   %-22s %6s       %7s       | %6s       %7s\n' "baseline ObjC app" "$base3" "$(bytes "$WORK/base-3.txt")" "$base13" "$(bytes "$WORK/base-13.txt")"
if [ "$app13" -gt "$base13" ] || [ "$app13" -gt "$app3" ]; then
    echo "   FAIL: the app leaks beyond the AppKit baseline. Roots:"
    grep -E "ROOT LEAK|ROOT CYCLE" "$WORK/app-13.txt" | head -20
    status=1
else
    echo "   OK: no leaks beyond the AppKit baseline, no growth over time"
fi

exit $status
