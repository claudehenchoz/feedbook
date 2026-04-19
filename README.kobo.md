# Running Feedbook on Kobo

This guide walks you through installing Feedbook on a Kobo e-reader so you can generate EPUBs from RSS/Atom feeds directly on-device — no computer needed after setup. Tap a menu item, wait a few seconds, and your reader fills with fresh articles.

## Prerequisites

You must install **NickelMenu** first. NickelMenu is a launcher that adds custom entries to your Kobo's main menu, which is how you'll trigger Feedbook runs.

Follow the official installation instructions here: <https://pgaskin.net/NickelMenu/#install>

Come back once NickelMenu is installed and confirmed working (you should see its menu entries in the Kobo UI).

## Installation

Connect your Kobo to your computer via USB. The device mounts as a disk. All paths below are relative to the root of that mounted volume (`/mnt/onboard/` on the device itself).

### 1. Create the Feedbook folder

Create a folder at `.adds/feedbook/` on the Kobo's storage.

### 2. Copy the binary and run script

Into `.adds/feedbook/`, copy:

- The Kobo Feedbook binary (built for `armv7-unknown-linux-musleabihf` — see the main README's "Build for Kobo" section). The file should be named `feedbook` with no extension.
- The `run.sh` script shown below.

**`run.sh`**

```sh
#!/bin/sh
# 1. Show a toast that it started (3000 = 3 seconds)
qndb -m mwcToast 3000 "Feedbook is downloading articles..."
# 2. Run your Rust app
/mnt/onboard/.adds/feedbook/feedbook
# 3. Check if successful
if [ $? -eq 0 ]; then
    # Make sure the file is actually written to the disk before scanning
    sync
    # Show your success message!
    qndb -m mwcToast 3000 "Feedbook completed successfully!"
    # Safely trigger the native "Importing content..." screen.
    # The -t 30000 and -s wait for the import sequence to finish before closing the script.
    qndb -t 30000 -s pfmDoneProcessing -m pfmRescanBooksFull
else
    # Show an error message if the Rust app panicked or failed
    qndb -m mwcToast 4000 "Feedbook failed to update."
fi
```

If you create `run.sh` on Windows, make sure it's saved with Unix (LF) line endings — CRLF will break execution on the Kobo. Most good editors (VS Code, Notepad++, Sublime) let you set this explicitly.

### 3. Create the Feedbook config

Still in `.adds/feedbook/`, create a file named `feedbook.toml`. A minimal starting point:

```toml
[defaults]
outfolder       = "/mnt/onboard/_feedbook"
limit           = 20
kobo            = true
max_image_width = 460
stdout          = true
dbpath          = "/mnt/onboard/.adds/feedbook"

[[feeds]]
url = "https://hnrss.org/newest?points=100"
```

What each default does:

- `outfolder` — where generated KEPUBs are written. `/mnt/onboard/_feedbook` means they'll appear in a `_feedbook` folder at the root of your Kobo's library.
- `limit` — max articles per feed.
- `kobo = true` — produce `.kepub.epub` files so Kobo's enhanced reading features (stats, precise bookmarks) work.
- `max_image_width` — resize images wider than this to save space and render well on e-ink.
- `stdout = true` — plain log lines (no progress bars) since there's no interactive terminal on the device.
- `dbpath` — where the SQLite cache lives. Keeping it next to the binary means re-runs only fetch new articles.

Add as many `[[feeds]]` entries as you like. See the main `README.md` and `feedbook.example.toml` for all the per-feed options (custom selectors, per-feed output folders, etc.).

### 4. Create the NickelMenu entry

Create a file at `.adds/nm/feedbook` (no extension) with this single line:

```
menu_item :main :Feedbook :cmd_spawn :quiet:/bin/sh /mnt/onboard/.adds/feedbook/run.sh
```

This registers a "Feedbook" entry in the Kobo's main NickelMenu. `:cmd_spawn` runs the script in the background so the UI stays responsive, and `:quiet:` suppresses the default "command running" dialog in favor of the toast notifications `run.sh` produces.

### 5. Eject and run

Safely eject the Kobo from your computer. Once the device finishes re-scanning its library, you're ready to go.

1. Make sure the Kobo is connected to Wi-Fi. Feedbook needs network access to fetch feeds, article pages, images, and favicons.
2. Tap the NickelMenu icon (bottom-right corner of the home screen).
3. Tap **Feedbook**.

You'll see a toast: *"Feedbook is downloading articles..."*. Depending on how many feeds you have and how many new articles are waiting, this can take anywhere from a few seconds to a couple of minutes. When it finishes, another toast confirms success and the Kobo automatically re-scans its library — your new KEPUBs appear in the `_feedbook` collection on the home screen.

## Folder layout recap

After setup, the relevant parts of your Kobo's storage look like this:

```
.adds/
  feedbook/
    feedbook            ← the ARM binary
    feedbook.toml       ← your config
    feedbook.sql        ← SQLite cache (created on first run)
    run.sh              ← the launcher script
  nm/
    feedbook            ← NickelMenu entry
_feedbook/              ← generated .kepub.epub files land here
```

## Troubleshooting

- **Nothing happens when you tap the menu item.** Make sure `run.sh` has Unix line endings and that the binary at `.adds/feedbook/feedbook` is actually the ARM musl build, not an x86 build from your dev machine. You can verify by running `file feedbook` on your computer — it should report `ARM, EABI5`.
- **Toast says "Feedbook failed to update".** Re-run with `stdout = true` already set and connect the Kobo over USB afterward — any errors from the last run are visible if you SSH into the device (via KoboRoot / telnet tweaks), but the simplest check is whether Wi-Fi was connected and whether the feed URL is reachable.
- **New articles don't show up in the library.** The `pfmRescanBooksFull` call at the end of `run.sh` should trigger a full re-scan, but if the device was mid-sleep it can occasionally miss. Manually tap the home screen or reboot the Kobo to force a re-scan.
- **Cache grows too large.** Feedbook auto-prunes (500 articles per feed, 90-day TTL), so this shouldn't happen in practice. If it does, just delete `feedbook.sql` — the next run rebuilds it.
