# Troubleshooting

> Every item below comes from a **real problem this project actually hit**. Before anything else, read
> section 1: where the logs are and what to attach.
> 中文版：[`troubleshooting.zh-CN.md`](troubleshooting.zh-CN.md)

---

## 1. Where logs and settings live (start here)

```text
%APPDATA%\com.gotkicry.audiolink\
```

It contains `settings.json` (language / start on sign-in / last device), `trust.json` (pairing allow-list),
`identity\` (this device certificate and key), `.window-state.json` and the logs.

**When reporting a problem, attach**: (1) the version from the About panel, (2) the log file, (3) the first
three numbers of each device address, (4) the network type (cable / Wi-Fi / hotspot), (5) a telemetry
screenshot or exported CSV. Without these we can only guess.

---

## 2. Device not found / cannot connect (most common)

| Cause | How to confirm | What to do |
|---|---|---|
| **The two devices are on different subnets** | Compare the first three numbers of both addresses: `192.168.3.x` and `192.168.31.x` are two different subnets | Put them behind one router. Across subnets they neither discover nor reach each other (this project really hit it: phone `192.168.31.231`, PC `192.168.3.200`, ping 100% loss) |
| **AP client isolation** (the router keeps wireless clients apart) | Same Wi-Fi but ping fails both ways | Turn off "AP isolation / client isolation" on the router. Adding the peer address manually from its status strip usually still fails while isolation is on |
| **Windows Firewall** blocking UDP | You did not allow it on first run | Tick "Private networks" in the firewall prompt, or add it back under "Allow an app through Windows Firewall" |
| Phone hotspot / guest Wi-Fi / corporate network | - | Those shapes often isolate clients; try a normal home router |
| Port blocked or in use | default `58290` | Try another network; both devices must sit in the same one |

---

## 3. Connected but no sound

1. Is the peer card showing **Streaming**? Nothing plays before a stream is started.
2. Is the volume slider at 0 (range 0-200%)?
3. **Which output device is the PC capturing?** AudioLink captures what a chosen output device is playing, so
   the player must send audio to **that same** device. After changing the Windows default output, click
   "Refresh devices" and pick it again.
4. Players using **exclusive mode** (WASAPI Exclusive) are absent from the system mix and cannot be captured -
   turn exclusive mode off.
5. On the phone: adjust **media** volume (not ringtone or notification volume).

---

## 4. Choppy, stuttering or noisy audio

- Read the telemetry panel in this order: **loss, jitter p95, buffer level, underruns**;
- 2.4 GHz congestion is the usual culprit: switch to 5 GHz or put both ends behind one router;
- Buffer level pinned at the cap (about 120 ms) means heavy jitter; steadily growing underruns mean the link
  cannot keep up;
- **On weak links the bitrate drops automatically** (adaptive). That is by design, not a fault;
- Bluetooth headsets add 100-200 ms at the OS level, unrelated to AudioLink.

---

## 5. Android side

| Symptom | Cause and fix |
|---|---|
| `adb install` fails with **-99** (MIUI/HyperOS) | Enable "**Install via USB**" in Developer options (some builds call it "Install apps via USB"), then reinstall |
| "App not installed" | Wrong ABI: 32-bit devices need the `armeabi-v7a` package, nearly every modern phone needs `arm64-v8a` |
| Killed in the background, audio stops | Add AudioLink to the battery-optimisation allow-list; check the foreground service notification |
| Want the phone to **capture** system audio | Not implemented yet (see user guide section 13); Android currently receives and plays |

---

## 6. PC side

| Symptom | Explanation |
|---|---|
| Window closed but the app keeps running | **By design**: AudioLink lives in the tray (it may still be feeding another room). Right-click the tray icon, then Quit |
| SmartScreen warning on install | The installer is **not code-signed** (see [`docs/42`](../42-m5-release-pipeline.md) section 2.2); choose "More info - Run anyway" |
| Portable build says WebView2 is missing | Install the "Microsoft Edge WebView2 Runtime" once (bundled with Windows 11) |
| Start-on-sign-in does not work | (1) It is disabled under Windows "Startup apps"; (2) you use the portable build and **moved the folder** - auto-start registers the exe path of that moment, so toggle the setting once after moving |

---

## 7. Updates

- **In-app auto-update failing**: the publisher must ship `latest.json` plus a `.sig` signature in the Release
  (missing either one means clients will not update); the client must also be able to reach GitHub (proxies and
  corporate networks may block it).
- **Manual update**: installers simply overwrite (settings survive); for the portable build, close the app and
  replace the exe.
- **Does the allow-list survive an update?** Yes - settings live in the user profile (section 1), independent of
  the install shape.

---

## 8. Known limitations (not bugs, do not troubleshoot these)

- No routing across subnets or the public internet - out of scope, LAN only;
- The installer is unsigned, so SmartScreen warns on first run;
- No iOS or web client;
- On Windows, capture is the **whole output device mix**, not a single application;
- The portable build has no in-app auto-update (replace the exe manually).

---

## 9. Still stuck

Open an issue in the repository and attach the material from section 1. With that, almost every problem can
be located in one pass.
