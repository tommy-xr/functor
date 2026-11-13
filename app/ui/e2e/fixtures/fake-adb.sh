#!/bin/sh
# A fake `adb` playing one headset, for the cable-pairing tests. Its state
# is files beside it: `devices` (lines after "List of devices attached"),
# `pm` (what `pm list packages` prints), and `app/`, the headset app's
# private storage, where the `run-as` commands the desktop app runs are
# evaluated. `not-debuggable`, `write-fails`, `mangle` (a write lands,
# then something else replaces the file) and `lose-read-back` (a write
# lands, then the next read fails) make them fail. Each call's arguments
# are appended to `calls`.
root=$(cd "$(dirname "$0")" && pwd)
printf '%s\n' "$*" >> "$root/calls"
if [ "$1" = "-s" ]; then shift 2; fi
case "$1" in
  devices) echo "List of devices attached"; cat "$root/devices"; exit 0 ;;
  shell) shift ;;
  *) echo "fake adb: $*" >&2; exit 1 ;;
esac
cmd="$*"
case "$cmd" in
  "pm list packages "*) cat "$root/pm"; exit 0 ;;
  "run-as dev.functor.runner "*)
    if [ -e "$root/not-debuggable" ]; then
      echo "run-as: package not debuggable: dev.functor.runner" >&2; exit 1
    fi
    if [ -e "$root/fail-next-read" ]; then
      rm "$root/fail-next-read"; echo "error: closed" >&2; exit 1
    fi
    case "$cmd" in *" mv "*)
      if [ -e "$root/write-fails" ]; then
        cat > /dev/null; echo "mv: Read-only file system" >&2; exit 1
      fi ;;
    esac
    cd "$root/app" || exit 1
    eval "${cmd#run-as dev.functor.runner }"
    status=$?
    case "$cmd" in *" mv "*)
      if [ -e "$root/mangle" ]; then printf '{"device":"00"}' > files/device-link.json; fi
      if [ -e "$root/lose-read-back" ]; then : > "$root/fail-next-read"; fi ;;
    esac
    exit $status ;;
esac
echo "fake adb: unknown shell command: $cmd" >&2
exit 1
