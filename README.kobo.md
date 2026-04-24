# Running Feedbook on Kobo

Install Feedbook as a NickelMenu item and generate KEPUBs from RSS/Atom feeds directly on-device — no computer needed after setup. Tap a menu item, wait a few seconds, and your reader fills with fresh articles.

## Prerequisites

You need **NickelMenu** installed first. It's a launcher that adds custom entries to your Kobo's main menu, which is how you'll trigger Feedbook runs.

Install it from here: <https://pgaskin.net/NickelMenu/#install>

Don't proceed until NickelMenu is working (you should see its menu entries in the Kobo UI).

## Install

### 1. Download and unpack

Grab `feedbook-kobo-*.zip` from the [Releases page](../../releases) and unpack it. Inside you'll find an `.adds/` folder containing everything pre-assembled:

```
.adds/
  feedbook/
    feedbook          ← the ARM binary
    feedbook.toml     ← default config
    run.sh            ← launcher script
  nm/
    feedbook          ← NickelMenu entry
```

### 2. Copy to the Kobo

Connect the Kobo via USB. It mounts as a disk. Drag the `.adds/` folder from the zip onto the root of the Kobo, merging with the existing `.adds/` directory if NickelMenu created one.

### 3. Edit the config (optional but likely)

Open `.adds/feedbook/feedbook.toml` on the Kobo in a text editor and add the feeds you care about. A minimal starting point is included:

```toml
[defaults]
outfolder       = "/mnt/onboard/_feedbook"
limit           = 20
kobo            = true
max_image_width = 460
dbpath          = "/mnt/onboard/.adds/feedbook"

[[feeds]]
url = "https://hnrss.org/newest?points=100"
```

A few device-specific notes on these defaults:

- `outfolder = "/mnt/onboard/_feedbook"` puts generated files in a collection at the root of the Kobo library.
- `kobo = true` produces `.kepub.epub` so Kobo's enhanced reading features work (stats, precise bookmarks, highlights).
- `dbpath` points at the same folder as the binary so re-runs only fetch new articles.

For the full list of config keys and what they do, see the [main README](README.md#config-keys).

If you edit files on Windows, make sure your editor saves with **Unix (LF) line endings**. CRLF will break `run.sh` on the Kobo. Good editors (VS Code, Notepad++, Sublime) let you set this explicitly.

### 4. Run

Safely eject the Kobo and wait for the library re-scan to finish.

1. Make sure the Kobo is on Wi-Fi — Feedbook needs network access for feeds, articles, images, and favicons.
2. Tap the NickelMenu icon (bottom-right of the home screen).
3. Tap **Feedbook**.

You'll see a toast: *"Feedbook is downloading articles..."*. Depending on how many feeds and new articles you have, it takes a few seconds to a couple of minutes. When it finishes, another toast confirms success and the Kobo re-scans its library — new KEPUBs appear in the `_feedbook` collection.

## Troubleshooting

- **Nothing happens when you tap the menu item.** Make sure `run.sh` has Unix line endings and that the binary at `.adds/feedbook/feedbook` is the ARM musl build, not an x86 one. On your computer: `file feedbook` should report `ARM, EABI5`.

- **Toast says "Feedbook failed to update".** Confirm Wi-Fi is connected and your feed URLs are reachable. Enable `log = true` in `[defaults]` and re-run — `feedbook.log` next to the binary will show what went wrong.

- **New articles don't show up in the library.** The launcher script triggers a full re-scan, but if the device was mid-sleep it can occasionally miss. Tap the home screen or reboot the Kobo to force a re-scan.

- **Cache grows too large.** Feedbook auto-prunes (500 articles per feed, 90-day TTL), so this shouldn't happen in practice. If it does, delete `feedbook.sql` — the next run rebuilds it.
