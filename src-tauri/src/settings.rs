use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

fn default_port() -> u16 { 7842 }
fn default_origin() -> String { "*".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Lance le bridge au démarrage de Windows
    pub autostart: bool,

    /// Répertoire SaveGames de Foxhole (None = auto-détecté)
    pub sav_path: Option<String>,

    /// Jeton secret partagé avec le site web pour authentifier les connexions SSE
    pub token: String,

    /// Port d'écoute du serveur SSE local
    #[serde(default = "default_port")]
    pub port: u16,

    /// Origine autorisée pour CORS (ex. "https://foxwar.example.com" ou "*")
    #[serde(default = "default_origin")]
    pub allowed_origin: String,

    /// Démarre sans fenêtre (tray uniquement) lorsqu'il est lancé au démarrage
    /// de Windows. Sans effet sur un lancement manuel.
    #[serde(default)]
    pub silent_start: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            autostart: false,
            sav_path: default_sav_path(),
            token: generate_token(),
            port: default_port(),
            allowed_origin: default_origin(),
            silent_start: false,
        }
    }
}

impl Settings {
    /// Charge depuis le fichier ou retourne les valeurs par défaut.
    pub fn load() -> Self {
        match Self::config_path().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(json) => serde_json::from_str(&json).unwrap_or_default(),
            None => Self::default(),
        }
    }

    /// Persiste sur disque.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::config_path().ok_or("Cannot resolve config dir")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("foxwar-bridge").join("settings.json"))
    }
}

pub fn generate_token() -> String {
    Uuid::new_v4().to_string().replace('-', "")
}

/// Chemin par défaut du répertoire SaveGames de Foxhole.
fn default_sav_path() -> Option<String> {
    dirs::data_local_dir()
        .map(|d| d.join("Foxhole").join("Saved").join("SaveGames"))
        .map(|p| p.to_string_lossy().into_owned())
}
