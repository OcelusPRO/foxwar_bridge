# Foxwar Bridge

A lightweight Windows desktop app built with **Tauri 2 + Rust** that watches your Foxhole save files and streams updates to the [Foxwar](https://foxwar.net) web app via a locally-secured **SSE** feed.

When Foxhole writes a new `MapData.sav`, the bridge detects the change within 500 ms and pushes the file to the browser — no manual drag-and-drop required.

---

## How it works

```
Foxhole game
    │  writes MapData.sav
    ▼
%LOCALAPPDATA%\Foxhole\Saved\SaveGames\
    │  notify-debouncer-mini (500 ms)
    ▼
Foxwar Bridge (Tauri / Rust)
    │  base64-encodes the .sav
    ▼
http://127.0.0.1:<PORT>/sse?token=<TOKEN>   ← loopback only, token-gated
    │
    ▼
foxwar.net (browser)
    │  decodes base64 → Uint8Array → File
    ▼
ImportedPinsContext  →  map pins update
```

The SSE server is **off by default**. It starts only when the bridge receives a `foxwar://connect` deep link from the web app — meaning the user explicitly clicked "Connect bridge" in the settings page.

---

## Security

| Property | Detail |
|---|---|
| **Loopback-only** | The SSE server binds to `127.0.0.1`, never to a network interface. Remote machines cannot reach it. |
| **Token-gated** | Every SSE connection must include `?token=<TOKEN>` in the query string. Requests without a valid token receive `401 Unauthorized`. |
| **CORS origin** | Requests are validated against an allowed origin. Defaults to `*` for local development; should be set to `https://foxwar.net` in production. |
| **On-demand server** | The HTTP server does not run until the user explicitly connects from the web app via the `foxwar://` protocol. |

### Configuring the allowed origin

**From the web app** — the site sends a deep link when connecting:

```
foxwar://connect?port=7842&token=<TOKEN>&origin=https%3A%2F%2Ffoxwar.net
```

**Manually** — open a Run dialog (`Win+R`) and enter:

```
foxwar://configure?origin=https://foxwar.net
```

Accepted origins for development:

| Environment | Origin |
|---|---|
| Production | `https://foxwar.net` |
| Backend dev | `http://localhost:8080` |
| Vite dev server | `http://localhost:5173` |

To allow all origins (not recommended in production):

```
foxwar://configure?origin=*
```

---

## `foxwar://` protocol reference

| Deep link | Effect |
|---|---|
| `foxwar://connect?port=P&token=T&origin=O` | Start the SSE server on port `P` with token `T`, restrict CORS to origin `O` |
| `foxwar://configure?origin=O` | Update the allowed CORS origin without restarting the server |
| `foxwar://refresh` | Force-read and re-broadcast the latest SAV file immediately |
| `foxwar://open` | Bring the bridge window to the foreground |

The protocol is registered automatically by the installer. On first launch the bridge registers itself as the `foxwar://` handler for the current Windows user.

---

## SSE feed

The web app connects to:

```
GET http://127.0.0.1:7842/sse?token=<TOKEN>
```

Every time Foxhole saves, the bridge emits an event:

```json
{
  "type": "sav_updated",
  "timestamp": 1700000000,
  "path": "C:\\Users\\...\\123456789012345678_MapData.sav",
  "file": "<standard base64 of raw .sav bytes>"
}
```

The browser decodes `file` into a `Uint8Array`, wraps it in a `File`, and feeds it to the same import pipeline used for manual drag-and-drop.

Health check endpoint (no auth required):

```
GET http://127.0.0.1:7842/health
→ { "status": "ok", "clients": 1 }
```

---

## Features

| Feature | Implementation |
|---|---|
| Single instance | `tauri-plugin-single-instance` — second launch passes its args (including `foxwar://` URLs) to the running instance |
| Deep link protocol | `tauri-plugin-deep-link` + custom `protocol.rs` handler |
| File watcher | `notify-debouncer-mini` — 500 ms debounce on `*_MapData.sav` |
| SSE server | `axum 0.7` on `127.0.0.1:<port>` with token auth and CORS |
| On-demand server | Server starts/stops via `foxwar://connect`; `SseServerHandle` holds a `oneshot::Sender<()>` that shuts down axum when dropped |
| Tray icon | Close window → hide to tray, not quit. Right-click → Show / Quit |
| Autostart | `tauri-plugin-autostart` writes to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` |

---

## Tauri commands (bridge UI → Rust)

| Command | Description |
|---|---|
| `get_settings` | Returns current settings (sav path, origin, token, autostart) |
| `get_sse_status` | Returns `{ running, port, token, client_count }` |
| `set_autostart(enabled)` | Toggle Windows autostart |
| `set_sav_path(path)` | Change watched directory; restarts the file watcher |
| `set_allowed_origin(origin)` | Update CORS origin; takes effect on next server start |
| `regenerate_token()` | Generate a new random token; invalidates existing SSE connections |
| `pick_directory()` | Open a native folder picker dialog |
| `trigger_refresh()` | Force-broadcast the latest SAV file to all connected clients |

---

## Project structure

```
bridge/
├── .github/workflows/release.yml   ← CI: build on push to main, auto-tag, GitHub Release
├── package.json                    ← @tauri-apps/cli
├── package-lock.json
├── frontend/
│   └── index.html                  ← Bridge settings UI (autostart, SAV path, token, status)
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json             ← window config, bundle ID, deep-link schemes
    ├── capabilities/default.json  ← Tauri v2 permission grants
    └── src/
        ├── main.rs                 ← binary entry point
        ├── lib.rs                  ← Tauri setup, AppState, plugin registration
        ├── settings.rs             ← Settings struct, persisted to %APPDATA%\foxwar-bridge\settings.json
        ├── watcher.rs              ← notify file watcher, SAV regex, base64 broadcast
        ├── sse_server.rs           ← axum router, token middleware, ClientGuard, CORS
        ├── protocol.rs             ← foxwar:// URL parser and dispatcher
        ├── tray.rs                 ← system tray menu
        └── commands.rs             ← #[tauri::command] handlers
```

---

## Building

### Prerequisites

```powershell
# Rust (MSVC toolchain, Windows only)
rustup toolchain install stable-x86_64-pc-windows-msvc
rustup default stable-x86_64-pc-windows-msvc

# Node >= 18
npm install   # installs @tauri-apps/cli
```

### Development

```powershell
cd bridge
npx tauri dev
```

### Production build (MSI + NSIS installer)

```powershell
cd bridge
npx tauri build
# Outputs:
#   src-tauri/target/release/bundle/msi/*.msi
#   src-tauri/target/release/bundle/nsis/*.exe
```

---

## CI / Releases

Every push to `main` triggers the GitHub Actions workflow (`.github/workflows/release.yml`):

1. Reads `major.minor` from `Cargo.toml`
2. Finds the highest existing `v{major}.{minor}.*` git tag → increments patch (or starts at `.0`)
3. Patches the version in `Cargo.toml` and `tauri.conf.json`
4. Builds with `npx tauri build` on `windows-latest`
5. Creates a GitHub Release tagged `v{version}` with MSI and NSIS installers attached

Bumping `major` or `minor` in `Cargo.toml` before pushing resets the patch counter to `0`.

Download the latest release: **[github.com/OcelusPRO/foxwar_bridge/releases/latest](https://github.com/OcelusPRO/foxwar_bridge/releases/latest)**

---

## Manual protocol registration (dev / testing)

The installer handles this automatically. For manual registration:

```powershell
$exe = "$PWD\src-tauri\target\release\foxwar-bridge.exe"
reg add "HKCU\Software\Classes\foxwar"                   /ve /d "URL:Foxwar Bridge Protocol" /f
reg add "HKCU\Software\Classes\foxwar"                   /v  "URL Protocol" /d "" /f
reg add "HKCU\Software\Classes\foxwar\shell\open\command" /ve /d "`"$exe`" `"%1`"" /f
```

Verify:

```powershell
Start-Process "foxwar://open"
```
