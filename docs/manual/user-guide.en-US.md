# AudioLink User Guide

> For **users**. To change the code, see [`CONTRIBUTING.md`](../../CONTRIBUTING.md); design details live in [`docs/`](../).
> 中文版：[`user-guide.zh-CN.md`](user-guide.zh-CN.md)

---

## 1. What this is

AudioLink sends the audio already playing on one device to other devices on the same local network, with low
latency. It works both ways (PC ⇄ phone), to several devices at once, and can mix several incoming streams
into one output.

**It only works inside your LAN**: there is no server in the middle, no audio is uploaded anywhere, and no
telemetry leaves your machines.

---

## 2. Which build to install

| Build | Best for | Differences |
|---|---|---|
| **Windows installer** (`AudioLink_*_x64-setup.exe`) | most PC users | installs into Windows, Start Menu and uninstall entry, supports **in-app auto-update** |
| **Portable zip** (`AudioLink_*_x64_portable.zip`) | no-install use, USB sticks | unzip and run, delete the folder to uninstall; updates are manual; no auto-update |
| **Android APK** | phones / tablets | two packages: `arm64-v8a` (most phones) and `armeabi-v7a` (older 32-bit devices). The wrong one fails with "app not installed" |

> The portable build needs **WebView2**: Windows 11 has it built in. On Windows 10, install the
> "Microsoft Edge WebView2 Runtime" once if you are told it is missing (the installer does the same check).

---

## 3. First run: three steps

1. **Put both devices on the same LAN** (same Wi-Fi, or the same router with a cable).
2. **Connect from the PC**: in "Add a device manually", enter the other side address (like `192.168.1.23`
   or `192.168.1.23:58290`). The phone does not need to enter anything - one side starts the connection.
3. **Pair**: one screen shows 6 digits; type them on the other device. When both sides show the same digits,
   no third device sits in between.

After pairing, the device goes on the allow-list, so you will not need the code again.

---

## 4. Streaming and receiving

| Goal | How |
|---|---|
| Send this device audio out | Click "Start streaming" on the peer card |
| Stop | "Stop streaming" on the same card |
| Set the other side volume | The volume slider (0-200%); changes ramp over about 200 ms |
| Send to several devices | Start streaming on each card. Capture happens once and all receivers share it |
| Receive | Nothing to do: playback starts when the other side starts streaming |

---

## 5. Where the audio comes from (Windows capture)

In "Capture audio" you pick an **output device** (speakers / headphones / virtual sound card) - AudioLink
captures what that device is currently playing.

- "System default (used when streaming starts)" follows the current Windows default output;
- Your player must send audio to **the same** device, otherwise AudioLink captures silence (for example if
  the player uses exclusive mode or a different sound card);
- "Stop streaming before switching sources": switching while streaming would push misaligned audio to the
  other side, so it is blocked;
- After swapping headphones or a sound card, click "Refresh devices".

---

## 6. Settings

| Setting | What it does |
|---|---|
| **Interface language** | 中文 / English, applied immediately and remembered across restarts |
| **Start on sign-in** | Launch with Windows |
| **Reconnect to the last device on launch** | Only remembers addresses that **connected successfully**; failures stay silent and show a short status hint |

**Closing the window does not quit the app** - AudioLink lives in the system tray (by design: it may still be
feeding another room). To really quit: right-click the tray icon, then Quit.

---

## 7. Sync groups and multi-source alignment (advanced)

**Sync groups**: on one sender, tick several devices and create a group. All receivers in the group start
playback at the same agreed moment instead of "as soon as data arrives" - that is what makes several rooms
sound simultaneous. "Lead time" is how far ahead that moment is (200 ms by default; more headroom helps on
weak networks).

**Multi-source alignment**: when this machine receives several streams at once (two phones into one PC), the
alignment panel shows each source latest index, estimated index and arrival time, plus the current **spread**
(difference between per-source sample indices). A spread of one frame or less (960 samples / 20 ms) means
aligned.

The "Broadcast common epoch" button makes every sender re-base its index to the same origin. It only exists on
the mixing side, because the receiver is the reference here.

---

## 8. Reading the numbers (telemetry panel)

| Metric | Meaning | Typical |
|---|---|---|
| End-to-end | Estimated capture-to-playback latency | 80-110 ms on Wi-Fi is normal |
| Jitter / p95 | Variation of arrival intervals | lower is better; a large p95 means shaky Wi-Fi |
| Loss | Link-level packet loss | 0% is best; redundant send and NACK compensate |
| Bitrate | Actual send rate | about 320 kbps by default (redundant send included); drops on weak links |
| Buffer level | How many ms the receiver holds to absorb jitter | 20-120 ms; pinned at the cap means heavy jitter |
| Underruns | Times playback ran out of data | fewer is better; steady growth means the link or buffer cannot keep up |

"Export CSV" asks the backend to write the accumulated samples to a CSV file and prints the path - attach it
when reporting a problem.

---

## 9. Where settings and data live

All of it is in your **user profile**, not next to the executable:

```text
%APPDATA%\com.gotkicry.audiolink\
  settings.json         language / start on sign-in / last device
  trust.json            paired-device allow-list
  identity\             this device self-signed certificate and key (the basis of pairing trust)
  .window-state.json    window position and size
  logs                  look here when something goes wrong
```

The portable build **shares** this profile with the installer: install first, switch to portable later, and
your settings and allow-list are still there. To start over, quit the app and delete the whole
`com.gotkicry.audiolink` folder.

> Deleting `identity\` is the same as changing your identity - previously paired devices must pair again.

---

## 10. Uninstalling

- **Installer**: Windows Settings, Apps, uninstall (or the Start Menu entry);
- **Portable**: delete the extracted folder; to remove settings too, delete the profile folder from section 9;
- **Android**: long-press the app icon, then uninstall.

---

## 11. When something goes wrong

See [`troubleshooting.en-US.md`](troubleshooting.en-US.md). The most common one first:
**if the two devices are not on the same subnet, they can neither discover nor reach each other** - compare
the first three numbers of both addresses.

---

## 12. Licence and third-party notices

AudioLink is released under **Apache-2.0** (see `LICENSE` and `NOTICE` in the repository). It bundles many
open-source components; the full list and licence texts ship with the installer: in the app, open
**About / third-party notices** and click Show. In the portable build the file sits next to the executable;
on Android it is the "Open source licences" page.

---

## 13. Not built yet (so you do not go looking)

- Android **system audio capture** and microphone input: **implemented** (loopback needs Android 10+, via MediaProjection + AudioPlaybackCapture; microphone via AudioRecord); **on-device acceptance is still outstanding**;
- Android **battery-optimisation guidance**: **implemented** (detection + actionable prompt; only opens the system allowlist, requests no sensitive permission). Android **start-on-boot is deliberately not implemented** — Android 15+ forbids starting a mediaPlayback foreground service from BOOT_COMPLETED (see docs/53-m5-audit.md);
- The Windows installer is **not code-signed**: SmartScreen warns on first run, choose "Run anyway";
- Across the public internet, iOS and a web client: explicitly out of scope.
