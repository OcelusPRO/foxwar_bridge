/**
 * Module Gradle : foxwar-bridge
 *
 * Tâches disponibles :
 *   ./gradlew :bridge:installDeps  → installe les dépendances npm (Tauri CLI)
 *   ./gradlew :bridge:build        → compile le bridge (MSI/NSIS) sur Windows
 *   ./gradlew :bridge:deploy       → copie l'installeur dans app/src/main/resources/bridge/
 *
 * ⚠ La compilation nécessite Windows + la chaîne MSVC + Rust 1.80+.
 *   Elle est intentionnellement séparée du build Docker (Linux).
 */

val bridgeDir: File = projectDir
val backendBridgeResources: File = rootProject.file("app/src/main/resources/bridge")

// ─── Dépendances npm (Tauri CLI) ─────────────────────────────────────────────

tasks.register<Exec>("installDeps") {
    group = "bridge"
    description = "Installe les dépendances npm du bridge (Tauri CLI)"
    workingDir = bridgeDir
    commandLine(npmCmd(), "install", "--prefer-offline")
}

// ─── Compilation Tauri / Rust ─────────────────────────────────────────────────

tasks.register<Exec>("build") {
    group = "bridge"
    description = "Compile Foxwar Bridge (Tauri + Rust) — requiert Windows + Rust MSVC"
    dependsOn("installDeps")
    workingDir = bridgeDir

    commandLine(npxCmd(), "tauri", "build")

    doFirst {
        logger.lifecycle("⚙  Compilation du bridge Tauri (cela peut prendre plusieurs minutes)…")
        check(org.gradle.internal.os.OperatingSystem.current().isWindows) {
            "La compilation du bridge Tauri nécessite Windows. Utilisez la CI Windows ou compilez manuellement."
        }
    }

    doLast {
        logger.lifecycle("✔  Bridge compilé avec succès.")
    }
}

// ─── Déploiement vers les ressources backend ──────────────────────────────────

tasks.register<Copy>("deploy") {
    group = "bridge"
    description = "Copie l'installeur bridge vers app/src/main/resources/bridge/"
    dependsOn("build")

    val bundleDir = bridgeDir.resolve("src-tauri/target/release/bundle")

    from(bundleDir) {
        // MSI (Windows Installer) ou NSIS (exe auto-extractible)
        include("msi/*.msi", "nsis/*.exe")
        // Renomme en "foxwar-bridge-setup.{msi|exe}" pour un nom stable côté backend
        eachFile {
            val ext = name.substringAfterLast('.')
            relativePath = RelativePath(true, "foxwar-bridge-setup.$ext")
        }
    }

    into(backendBridgeResources)
    includeEmptyDirs = false

    doFirst {
        backendBridgeResources.mkdirs()
        logger.lifecycle("Déploiement des artefacts vers $backendBridgeResources")
    }
}

// ─── Tâche de nettoyage ───────────────────────────────────────────────────────

tasks.register<Delete>("cleanBridge") {
    group = "bridge"
    description = "Supprime les artefacts de compilation Rust"
    delete(bridgeDir.resolve("src-tauri/target"))
}

// ─── Helpers cross-platform pour npm / npx ───────────────────────────────────

fun npmCmd() = if (org.gradle.internal.os.OperatingSystem.current().isWindows) "npm.cmd" else "npm"
fun npxCmd() = if (org.gradle.internal.os.OperatingSystem.current().isWindows) "npx.cmd" else "npx"
