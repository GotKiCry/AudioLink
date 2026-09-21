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
2. **Choose the host on the receiver**: select the computer under "Local hosts" on your phone. On another PC,
   first expand "Listen to another device". The host usually appears within a few seconds of opening AudioLink.
   If it is missing, enter its displayed address manually, such as `192.168.1.23:58290`.
   **The host enters no address**; it waits for the receiver to connect.
3. **Pair**: the host screen shows 6 digits; type them on the receiver. When both sides show the same digits,
   no third device sits in between.

After pairing, the device goes on the allow-list, so you will not need the code again.

> **The receiver connects to the host.** Choose a host, enter a manual address, and type the pairing code on the listening device.

Hosts remain discoverable while streaming or paused. Offline hosts disappear after about 10 seconds; discovery does not grant trust.
Android scans while the receiving entry is visible and the app is in the foreground. Backgrounding stops discovery, while existing audio connections continue.
Choose "Refresh hosts" to clear old results and restart scanning. To make a phone discoverable as a host, select an audio source under "Send audio" and start its service. Phones used only as receivers are not listed as hosts.
If no host appears, check that both devices are on the same subnet, guest/client isolation is off, and Windows allows AudioLink on private networks.
Discovery uses UDP 58280; audio connections use UDP 58290 by default. Manual address entry remains available.

---

## 4. Streaming and receiving

| Goal | How |
|---|---|
| Send this device audio out | Streaming starts automatically once a device connects and pairs |
| Pause / resume | Use "Pause streaming" / "Resume streaming" in the sidebar; pairing is retained |
| Set the other side volume | The volume slider (0-200%); changes ramp over about 200 ms |
| Connect several devices | The desktop shell currently sends to one device at a time. Pause, then choose "Resume streaming" on another device row |
| Receive | Nothing to do: playback starts when the other side starts streaming |

The sidebar contains sharing controls and the audio source. The main area shows your address and connected devices.
Expand "Listen to another device" to connect to another host. "Waiting for a device" means no audio is being sent;
"Streaming audio" appears when sending begins. A manual pause applies to this sharing session: new connections,
reconnections, and changes to the automatic streaming preference do not override it. Choose "Resume streaming" to continue.
Restarting AudioLink uses your saved automatic streaming preference again. Pairing and audio source setup must finish
before streaming starts. If startup fails, resolve the displayed error and retry.

---

## 5. Where the audio comes from (Windows capture)

In "Audio source" in the sidebar you pick an **output device** (speakers / headphones / virtual sound card) - AudioLink
captures what that device is currently playing.

- "System default (used when streaming starts)" follows the current Windows default output;
- Your player must send audio to **the same** device, otherwise AudioLink captures silence (for example if
  the player uses exclusive mode or a different sound card);
- Pause streaming to change sources; resuming uses the newly selected device;
- After swapping headphones or a sound card, click "Refresh devices".

---

## 6. Settings

| Setting | What it does |
|---|---|
| **Stream automatically when a device connects** | On by default; turn off to start manually. Manual pause is independent of this preference |
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

## 13. Built / not built (so you do not go looking)

- Android **system audio capture** and microphone input: **implemented** (loopback needs Android 10+, via MediaProjection + AudioPlaybackCapture; microphone via AudioRecord); **on-device acceptance is still outstanding**;
- Android **battery-optimisation guidance**: **implemented** (detection + actionable prompt; only opens the system allowlist, requests no sensitive permission). Android **start-on-boot is deliberately not implemented** — Android 15+ forbids starting a mediaPlayback foreground service from BOOT_COMPLETED (see docs/53-m5-audit.md);
- The Windows installer is **not code-signed**: SmartScreen warns on first run, choose "Run anyway";
- **In-app updates**: available on both desktop and Android (checked **only when you ask**, never downloaded or installed in the background); the first Android update needs you to allow "install unknown apps";
- Across the public internet, iOS and a web client: explicitly out of scope.

---

## 14. Updates

**Updates only happen when you ask.** AudioLink never downloads or installs in the background: being
restarted silently while you are streaming or recording is not acceptable.

**Desktop**: open the "Software update" panel, click **Check for updates**, then **Download and install**.
The installer first winds the session down gracefully, closes AudioLink, runs, and reopens the app.

**Android**: top bar → **Settings** → **Software update** → **Check for updates** → **Download and install**.

- The first Android update asks the system for permission to install apps - grant it, come back, tap **Continue install**;
- After that, some vendors add one more **hand-off confirmation** (e.g. ColorOS asking whether AudioLink may open the installer) - just allow it; that is a system guard, not an error;
- Android only sees **published** releases: a freshly built version that nobody has published yet still shows as "up to date";
- Android can only install over an existing copy from the second version on (Android requires the same signature).

When an update fails, the panel states the reason (cannot reach GitHub, timeout, wrong package). If GitHub is
unreliable on your network, simply retry later.

### Android console

- **Receive** opens by default. Enter the host address, then the pairing code on your first connection. Once connected, playback status, media volume and disconnect controls appear. Each device also has its own mute, gain and disconnect controls.
- **Send** shows this device's address, the listener control and audio source. Switching views does not start or stop audio; changing the capture source still restarts the service and interrupts sessions.
- **Settings** contains appearance, background activity, updates, open-source licences and collapsed diagnostics. Back returns through each page and preserves your address and scroll position.
