# Running Feedbook on Kindle

This guide walks you through installing Feedbook on a jailbroken Kindle e-reader so you can generate EPUBs from RSS/Atom feeds directly on-device — no computer needed after setup. Tap a menu item, wait a few seconds, and your reader fills with fresh articles.

## Prerequisites

Two things need to be installed on your Kindle before Feedbook will work.

### 1. A jailbroken Kindle

Feedbook requires shell access and the ability to run custom binaries, which stock Kindle firmware doesn't permit. The jailbreak process varies by model and firmware version; rather than reproduce instructions that go stale quickly, start here:

<https://wiki.mobileread.com/wiki/Kindle_Hacks_Information>

Follow whichever guide matches your model and current firmware version. Don't proceed with this guide until your Kindle is jailbroken and you can install custom extensions.

### 2. KUAL (Kindle Unified Application Launcher)

KUAL is a launcher that adds a menu of custom extensions to your Kindle, which is how you'll trigger Feedbook runs.

Install KUAL following the instructions on MobileRead. You should see a "KUAL" book in your library that opens into a menu of installed extensions when tapped.

### 3. KOReader (strongly recommended)

Stock Kindle firmware reads MOBI/AZW3 but not EPUB. Feedbook produces EPUB files, so unless you have a recent Kindle firmware that transparently converts EPUBs on-device, you'll want **KOReader** — a third-party reader that handles EPUBs natively and renders them beautifully on e-ink.

KOReader installs as a KUAL extension. See: <https://github.com/koreader/koreader/wiki/Installation-on-Kindle-devices>

## Which zip do I need?

The Kindle architecture changed in late 2018. Pick the zip that matches your device:

| Zip                                           | Compatible models                                                  |
|-----------------------------------------------|--------------------------------------------------------------------|
| `feedbook-kindle-paperwhite3-and-older-*.zip` | Paperwhite 1/2/3, Voyage, Oasis 1, Basic (gen 7)                   |
| `feedbook-kindle-paperwhite4-and-newer-*.zip` | Paperwhite 4/5, Oasis 2/3, Basic (gen 10+), Scribe                 |

If in doubt, check your model: **Settings → Device Options → Device Info**, then look up the model number on MobileRead. Running the wrong binary produces an `Exec format error` — harmless, but nothing will happen.

## Installation

Connect your Kindle to your computer via USB. The device mounts as a disk. All paths below are relative to the root of that mounted volume (`/mnt/us/` on the device itself).

### 1. Unpack the release zip

Download the zip matching your model from the [Releases page](../../releases) and unpack it. Inside you'll find an `extensions/feedbook/` folder with everything pre-assembled:

```
extensions/
  feedbook/
    config.xml          ← KUAL extension manifest
    menu.json           ← menu entries
    feedbook            ← the ARM binary
    feedbook.toml       ← default config
    bin/
      feedbook.sh       ← launcher script
```

### 2. Copy the extension folder

Drag the entire `extensions/feedbook/` folder onto your Kindle, merging it into the existing `/mnt/us/extensions/` directory (which is where KUAL scans for installed extensions).

**Important — case sensitivity:** the folder must be lowercase `extensions`, not `EXTENSIONS`. Windows shows the drive case-insensitively, but Kindle's underlying filesystem is case-sensitive and KUAL only matches the lowercase name.

### 3. Edit the Feedbook config

Open `/mnt/us/extensions/feedbook/feedbook.toml` in a text editor. A minimal starting point is included in the zip:

```toml
[defaults]
outfolder       = "/mnt/us/documents/feedbook"
limit           = 20
max_image_width = 460
stdout          = true
dbpath          = "/mnt/us/extensions/feedbook"

[[feeds]]
url = "https://hnrss.org/newest?points=100"
```

What each default does:

- `outfolder` — where generated EPUBs are written. `/mnt/us/documents/feedbook` means they'll appear in a `feedbook` subfolder of your Kindle library.
- `limit` — max articles per feed.
- `max_image_width` — resize images wider than this to save space and render well on e-ink.
- `stdout = true` — plain log lines (no progress bars) since there's no interactive terminal on the device.
- `dbpath` — where the SQLite cache lives. Keeping it next to the binary means re-runs only fetch new articles.

Note the absence of `kobo = true` — that flag produces Kobo-specific `.kepub.epub` files, which Kindle readers don't benefit from. Feedbook produces standard EPUB on Kindle.

Add as many `[[feeds]]` entries as you like. See the main `README.md` and `feedbook.example.toml` for all the per-feed options (custom selectors, per-feed output folders, etc.).

If you edit `feedbook.toml` or `bin/feedbook.sh` on Windows, make sure your editor saves with Unix (LF) line endings — CRLF will break execution on the Kindle. Most good editors (VS Code, Notepad++, Sublime) let you set this explicitly.

### 4. Eject and run

Safely eject the Kindle from your computer.

1. Make sure the Kindle is connected to Wi-Fi. Feedbook needs network access to fetch feeds, article pages, images, and favicons.
2. Open the **KUAL** book from your library.
3. Tap **Feedbook** in the KUAL menu.
4. Tap **Run Feedbook**.

You'll see a status message drawn on-screen by `eips`: *"Feedbook: fetching articles..."*. Depending on how many feeds you have and how many new articles are waiting, this can take anywhere from a few seconds to a couple of minutes. When it finishes, the screen updates with a completion message and the Kindle re-scans its library.

Your new EPUBs appear in the `feedbook` folder of your library. Open them with KOReader (long-press the book on the home screen and pick "Open with KOReader" if you have the integration set up, or launch KOReader from KUAL and navigate to `/mnt/us/documents/feedbook/`).

## Folder layout recap

After setup, the relevant parts of your Kindle's storage look like this:

```
extensions/
  feedbook/
    config.xml          ← KUAL extension manifest
    menu.json           ← menu entries
    feedbook            ← the ARM binary
    feedbook.toml       ← your config
    feedbook.sql        ← SQLite cache (created on first run)
    bin/
      feedbook.sh       ← launcher script
documents/
  feedbook/             ← generated .epub files land here
```

## Troubleshooting

- **Feedbook doesn't appear in the KUAL menu.** Three things to check, in order:
  1. The folder is at `/mnt/us/extensions/feedbook/` with **lowercase** `extensions`.
  2. `config.xml` is present alongside `menu.json`. KUAL uses `config.xml` as the extension manifest; without it, the folder is invisible to KUAL regardless of what's inside.
  3. `menu.json` is valid JSON. A trailing comma or syntax error causes KUAL to silently drop the extension. Paste the file into a JSON validator to confirm.

  After any change, fully exit KUAL (back to the home screen) and reopen it — KUAL caches the extension list at startup. On some firmware versions you may need to eject USB first to trigger a filesystem remount.

- **Tapping the menu item does nothing / fails immediately.** Make sure `bin/feedbook.sh` has Unix line endings (not CRLF) and that the binary at `feedbook` is the right one for your Kindle's architecture. You can verify on your computer by running `file feedbook` — it should report `ARM, EABI5` for pre-2018 Kindles or `ARM aarch64` for 2018-and-newer ones. Running the wrong one produces `Exec format error` in the log.

- **Status message says Feedbook failed.** A `feedbook.log` file is written to `/mnt/us/extensions/feedbook/` on every run. Connect the Kindle over USB and open that file to see what went wrong — usually it's a Wi-Fi issue, an unreachable feed URL, or a typo in `feedbook.toml`.

- **New articles don't show up in the library.** The launcher script pokes the library service to trigger a rescan, but this is firmware-dependent and occasionally misses. If articles are on disk in `/mnt/us/documents/feedbook/` but not in the library, the most reliable fix is to restart the Kindle (**Menu → Settings → Menu → Restart**). You only need to do this once — subsequent runs will be picked up normally.

- **EPUBs don't open, or open but look terrible.** Stock Kindle firmware has spotty EPUB support. Install KOReader (see Prerequisites above) and open Feedbook-generated files through it. Rendering is dramatically better and the reading experience matches what you'd expect from a dedicated EPUB reader.

- **Cache grows too large.** Feedbook auto-prunes (500 articles per feed, 90-day TTL), so this shouldn't happen in practice. If it does, just delete `feedbook.sql` — the next run rebuilds it.
