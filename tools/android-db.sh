#!/bin/sh
# Put the Bible database into the Android app's assets: the APK carries it
# (its zip compression packs it to a third), and the app unpacks it on first
# launch. Run before `cargo apk build -p leverbe-android`.
#
# Not gzipped: aapt would quietly unzip a .gz asset and store it under the
# name without the suffix.
#
#   tools/android-db.sh [path/to/bible.db]   (default: data/bible.db)
set -eu
cd "$(dirname "$0")/.."
db="${1:-data/bible.db}"
if [ ! -f "$db" ]; then
    echo "$db does not exist: build it with  cargo run --release --bin import" >&2
    exit 1
fi
# A database left mid-transaction would ship without its journal.
if [ -s "$db-wal" ] || [ -s "$db-journal" ]; then
    echo "$db has a journal beside it: open it once with the app or the importer to settle it" >&2
    exit 1
fi
mkdir -p android/assets
cp "$db" android/assets/bible.db.part
mv android/assets/bible.db.part android/assets/bible.db
ls -l android/assets/bible.db
