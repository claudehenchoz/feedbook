#!/bin/sh
EXT_DIR="/mnt/us/extensions/feedbook"
BIN="${EXT_DIR}/feedbook"
CFG="${EXT_DIR}/feedbook.toml"

# eips prints to the e-ink screen at (col, row). -c clears.
eips 2 2 "Feedbook: fetching articles..."

MODE="$1"
EXTRA=""
[ "$MODE" = "force" ] && EXTRA="--force"

cd "${EXT_DIR}"
"${BIN}" --config "${CFG}" ${EXTRA} > "${EXT_DIR}/feedbook.log" 2>&1
RC=$?

sync

if [ $RC -eq 0 ]; then
    eips 2 2 "Feedbook: done. Triggering library scan..."
    # Kindle library rescan: touching a dummy file in documents/ and
    # running dbus-send to trigger the cc.scan event is the common pattern.
    # Simplest reliable approach: dbus poke.
    dbus-send --system /default com.lab126.powerd.resuming int32:1 2>/dev/null
    eips 2 2 "Feedbook: complete.                    "
else
    eips 2 2 "Feedbook: FAILED (see feedbook.log).   "
fi
