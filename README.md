# Foxwar Bridge

Application de bureau locale (Windows) compilée avec **Tauri 2 + Rust** qui surveille
le fichier `MapData.sav` de Foxhole et diffuse les mises à jour vers le site web via un
flux **SSE** sécurisé par jeton.

---

## Fonctionnalités

| Fonctionnalité | Détail |
|---|---|
| Instance unique | `tauri-plugin-single-instance` — une seule instance à la fois |
| Protocole `foxwar://` | Géré par `tauri-plugin-deep-link` + handler custom |
| Surveillance fichier | `notify-debouncer-mini` — debounce 500 ms sur `%LOCALAPPDATA%\Foxhole\Saved\SaveGames\` |
| Serveur SSE | `axum` sur `http://127.0.0.1:7842/sse?token=<TOKEN>` |
| Tray icon | Fermer la fenêtre → masquer dans le tray, pas quitter |
| Lancement démarrage | `tauri-plugin-autostart` (registre Windows `HKCU\...\Run`) |

---

## Prérequis

```
rustup toolchain install stable-x86_64-pc-windows-msvc
rustup default stable-x86_64-pc-windows-msvc
node >= 18
npm >= 9
```

Installer le CLI Tauri (une seule fois) :

```powershell
npm install          # dans ce dossier bridge/
```

---

## Icônes (requis avant le premier build)

Placez une image carrée `icon.png` (1024×1024 recommandé) dans `src-tauri/icons/`,
puis générez tous les formats :

```powershell
npx tauri icon src-tauri/icons/icon.png
```

Cela créera `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.ico`, `icon.icns`.

---

## Compilation

### Via Gradle (recommandé)

```powershell
# Depuis la racine du projet
./gradlew :bridge:build    # compile
./gradlew :bridge:deploy   # copie l'installeur dans app/src/main/resources/bridge/
```

Le `:app:build` suivant intègrera l'installeur dans le JAR backend.

### Directement avec Tauri

```powershell
cd bridge
npx tauri build            # MSI + NSIS dans src-tauri/target/release/bundle/
```

### Mode développement

```powershell
cd bridge
npx tauri dev
```

---

## Enregistrement du protocole `foxwar://`

Le protocole est enregistré **automatiquement par l'installeur** (MSI/NSIS) généré par Tauri.

Pour un enregistrement manuel (dev / tests) :

```powershell
# Adapter le chemin vers le binaire
$exe = "$PWD\src-tauri\target\release\foxwar-bridge.exe"
reg add "HKCU\Software\Classes\foxwar"                  /ve /d "URL:Foxwar Bridge Protocol" /f
reg add "HKCU\Software\Classes\foxwar"                  /v "URL Protocol" /d "" /f
reg add "HKCU\Software\Classes\foxwar\shell\open\command" /ve /d "`"$exe`" `"%1`"" /f
```

Test rapide :

```powershell
Start-Process "foxwar://open"
```

---

## Architecture

```
bridge/
├── build.gradle.kts          ← tâches Gradle (build, deploy)
├── package.json              ← @tauri-apps/cli
├── frontend/
│   └── index.html            ← UI Tauri (paramètres, statut, jeton)
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json       ← fenêtre, protocole, bundle
    ├── capabilities/
    │   └── default.json      ← permissions Tauri v2
    └── src/
        ├── main.rs           ← point d'entrée binaire
        ├── lib.rs            ← setup Tauri, état partagé, plugins
        ├── settings.rs       ← Settings (JSON dans %APPDATA%\foxwar-bridge\)
        ├── watcher.rs        ← surveillance fichier SAV
        ├── sse_server.rs     ← serveur axum SSE
        ├── protocol.rs       ← handler foxwar://
        ├── tray.rs           ← icône de notification
        └── commands.rs       ← commandes Tauri → frontend
```

---

## Flux SSE

Le frontend web se connecte à :

```
GET http://127.0.0.1:7842/sse?token=<TOKEN>
```

Payload reçu lors d'un changement de fichier :

```json
{
  "type": "sav_updated",
  "timestamp": 1700000000,
  "path": "C:\\Users\\...\\12345678901234567_MapData.sav",
  "file": "<base64 du fichier .sav>"
}
```

Le client décode le base64 et traite le `.sav` exactement comme un glisser-déposer.

---

## Commandes Tauri

| Commande | Description |
|---|---|
| `get_settings` | Retourne les paramètres courants |
| `get_status` | Retourne l'état du watcher, clients SSE, etc. |
| `set_autostart(enabled)` | Active/désactive le démarrage automatique |
| `set_sav_path(path)` | Change le répertoire surveillé (redémarre le watcher) |
| `set_allowed_origin(origin)` | Change l'origine CORS (`*` ou URL précise) |
| `regenerate_token()` | Génère un nouveau jeton |
| `pick_directory()` | Ouvre un sélecteur de dossier natif |
| `trigger_refresh()` | Force la relecture et diffusion du fichier SAV |

---

## Sécurité

- Le serveur SSE n'écoute que sur `127.0.0.1` (loopback), jamais sur l'interface réseau.
- Toute connexion SSE doit présenter le bon jeton (`?token=...`).
- L'origine CORS peut être restreinte à `https://ton-site.com` via l'UI.
- Le protocole `foxwar://configure?origin=https://ton-site.com` permet de configurer
  l'origine depuis le site web (deep link).

---

## Intégration Docker / CI

La compilation Tauri ne peut pas se faire dans un conteneur Linux standard (GUI, WebView2…).
Le workflow recommandé :

1. Compiler sur un runner Windows CI (GitHub Actions `windows-latest`)
2. Exécuter `./gradlew :bridge:deploy`
3. Committer `app/src/main/resources/bridge/*.msi` (ou archiver l'artefact)
4. Le `Dockerfile` Linux reconstruit le backend avec l'installeur inclus dans le JAR

Exemple de step GitHub Actions :

```yaml
- name: Build Foxwar Bridge
  runs-on: windows-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
      with: { targets: x86_64-pc-windows-msvc }
    - run: ./gradlew :bridge:deploy
    - uses: actions/upload-artifact@v4
      with:
        name: bridge-installer
        path: app/src/main/resources/bridge/
```
