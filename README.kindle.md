# Running Feedbook on Kindle

Install Feedbook as a KUAL extension and generate EPUBs from RSS/Atom feeds directly on-device — no computer needed after setup. Tap a menu item, wait a few seconds, and your reader fills with fresh articles.

## Prerequisites

Three things need to be in place before Feedbook will work.

### 1. A jailbroken Kindle

Feedbook needs shell access and the ability to run custom binaries, which stock Kindle firmware doesn't permit. The jailbreak process varies by model and firmware version; rather than reproduce instructions that go stale quickly, start here:

<https://wiki.mobileread.com/wiki/Kindle_Hacks_Information>

Don't proceed until your Kindle is jailbroken and you can install custom extensions.

### 2. KUAL (Kindle Unified Application Launcher)

KUAL is a launcher that adds a menu of custom extensions to your Kindle — that's how you'll trigger Feedbook runs. Install it per the MobileRead instructions. You should see a "KUAL" book in your library that opens into a menu of installed extensions when tapped.

### 3. KOReader (strongly recommended)

Stock Kindle firmware reads MOBI/AZW3 but not EPUB. Feedbook produces EPUB, so unless your firmware transparently converts EPUBs on-device, install **KOReader** — a third-party reader that handles EPUBs natively and renders them beautifully on e-ink. It installs as a KUAL extension.

<https://github.com/koreader/koreader/wiki/Installation-on-Kindle-devices>

## Install

### 1. Pick the right zip

The Kindle architecture changed in late 2018. Pick the zip that matches your device:

| Zip                                           | Compatible models                                              |
|-----------------------------------------------|----------------------------------------------------------------|
| `feedbook-kindle-paperwhite3-and-older-*.zip` | Paperwhite 1/2/3, Voyage, Oasis 1, Basic (gen 7)               |
| `feedbook-kindle-paperwhite4-and-newer-*.zip` | Paperwhite 4/5, Oasis 2/3, Basic (gen 10+), Scribe             |

If in doubt, check **Settings → Device Options → Device Info** and look up the model number on MobileRead. Running the wrong binary produces an `Exec format error` — harmless, but nothing will happen.

### 2. Download and unpack

Grab the right zip from the [Releases page](../../releases) and unpack it. Inside you'll find an `extensions/feedbook/` folder with everything pre-assembled:

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

### 3. Copy to the Kindle

Connect the Kindle via USB. It mounts as a disk. Drag the `extensions/feedbook/` folder onto the Kindle, merging it into the existing `extensions/` directory where KUAL scans for installed extensions.

**Important — case sensitivity.** The folder must be lowercase `extensions`, not `EXTENSIONS`. Windows shows the drive case-insensitively, but the Kindle's underlying filesystem is case-sensitive and KUAL only matches the lowercase name.

### 4. Edit the config (optional but likely)

Open `extensions/feedbook/feedbook.toml` on the Kindle in a text editor and add the feeds you care about. A minimal starting point is included:

```toml
[defaults]
outfolder       = "/mnt/us/documents/feedbook"
limit           = 20
max_image_width = 460
dbpath          = "/mnt/us/extensions/feedbook"

[[feeds]]
url = "https://hnrss.org/newest?points=100"
```

A few device-specific notes on these defaults:

- `outfolder = "/mnt/us/documents/feedbook"` puts generated files in a `feedbook` subfolder of the Kindle library.
- No `kobo = true` — that flag produces Kobo-specific `.kepub.epub` files that Kindle readers don't benefit from. Feedbook produces standard EPUB on Kindle.
- `dbpath` points at the same folder as the binary so re-runs only fetch new articles.

For the full list of config keys and what they do, see the [main README](README.md#config-keys).

If you edit files on Windows, make sure your editor saves with **Unix (LF) line endings**. CRLF will break `feedbook.sh` on the Kindle. Good editors (VS Code, Notepad++, Sublime) let you set this explicitly.

### 5. Run

Safely eject the Kindle.

1. Make sure the Kindle is on Wi-Fi — Feedbook needs network access for feeds, articles, images, and favicons.
2. Open the **KUAL** book from your library.
3. Tap **Feedbook**, then **Run Feedbook**.

You'll see a status message drawn on-screen: *"Feedbook: fetching articles..."*. Depending on how many feeds and new articles you have, it takes a few seconds to a couple of minutes. When it finishes, the screen updates with a completion message and the Kindle re-scans its library.

Your new EPUBs appear in the `feedbook` folder of your library. Open them with KOReader.

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

- **Feedbook doesn't appear in the KUAL menu.** Check, in order:
  1. The folder is at `/mnt/us/extensions/feedbook/` with **lowercase** `extensions`.
  2. `config.xml` is present alongside `menu.json`. KUAL uses `config.xml` as the extension manifest; without it, the folder is invisible regardless of what's inside.
  3. `menu.json` is valid JSON. A trailing comma or syntax error causes KUAL to silently drop the extension — paste it into a JSON validator to confirm.

  After any change, fully exit KUAL (back to the home screen) and reopen it — KUAL caches the extension list at startup. On some firmware versions you may need to eject USB first to trigger a filesystem remount.

- **Tapping the menu item does nothing or fails immediately.** Make sure `bin/feedbook.sh` has Unix line endings (not CRLF) and that the binary at `feedbook` is the right one for your Kindle's architecture. Verify on your computer: `file feedbook` should report `ARM, EABI5` for pre-2018 Kindles or `ARM aarch64` for 2018-and-newer ones. Running the wrong one produces `Exec format error` in the log.

- **Status message says Feedbook failed.** A `feedbook.log` file is written to `/mnt/us/extensions/feedbook/` on every run. Connect over USB and open it to see what went wrong — usually Wi-Fi, an unreachable feed URL, or a typo in `feedbook.toml`.

- **New articles don't show up in the library.** The launcher script pokes the library service to trigger a rescan, but this is firmware-dependent and occasionally misses. If articles are on disk in `/mnt/us/documents/feedbook/` but not in the library, restart the Kindle (**Menu → Settings → Menu → Restart**). You only need to do this once; subsequent runs are picked up normally.

- **EPUBs don't open, or open but look terrible.** Stock Kindle firmware has spotty EPUB support. Install KOReader (see Prerequisites) and open Feedbook-generated files through it — rendering is dramatically better.

- **Cache grows too large.** Feedbook auto-prunes (500 articles per feed, 90-day TTL), so this shouldn't happen in practice. If it does, delete `feedbook.sql` — the next run rebuilds it.
