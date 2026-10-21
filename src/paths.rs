use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static APP_ROOT_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Returns the root directory of the application assets and resources.
/// - In release/installed mode: Directory containing the executable.
/// - In development mode: Navigates upwards if running from target/debug or target/release.
pub fn app_root_dir() -> &'static PathBuf {
    APP_ROOT_DIR.get_or_init(|| {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        // If running inside target/debug or target/release, check if parent is project root
        if exe_dir.ends_with("debug") || exe_dir.ends_with("release") {
            if let Some(target_parent) = exe_dir.parent().and_then(|p| p.parent()) {
                if target_parent.join("characters").exists()
                    || target_parent.join("Cargo.toml").exists()
                {
                    return target_parent.to_path_buf();
                }
            }
        }

        // If current working directory has characters or Cargo.toml, prefer current directory
        if let Ok(cwd) = std::env::current_dir() {
            if cwd.join("characters").exists() || cwd.join("Murasame_live2d").exists() {
                return cwd;
            }
        }

        exe_dir
    })
}

/// Resolves an asset or model resource path.
/// If `path` is absolute, it returns the absolute path.
/// If `path` is relative, it checks `app_root_dir().join(path)`.
pub fn resolve_resource_path(path: impl AsRef<Path>) -> PathBuf {
    let p = path.as_ref();
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        let root_candidate = app_root_dir().join(p);
        if root_candidate.exists() {
            root_candidate
        } else {
            // Fallback to relative to current working directory
            p.to_path_buf()
        }
    }
}

/// Returns the user data directory where dynamic and mutable data should be stored.
/// Hierarchy:
/// 1. `MURASAME_DATA_DIR` environment variable.
/// 2. Portable mode: `./data` if directory exists or `portable` file is present.
/// 3. OS standard data directory:
///    - Windows: `%APPDATA%/murasame` (e.g. `C:\Users\<user>\AppData\Roaming\murasame`)
///    - Linux: `~/.local/share/murasame` or `$XDG_DATA_HOME/murasame`
///    - macOS: `~/Library/Application Support/murasame`
pub fn data_dir() -> &'static PathBuf {
    DATA_DIR.get_or_init(|| {
        // 1. Environment variable override
        if let Ok(env_path) = std::env::var("MURASAME_DATA_DIR") {
            let p = PathBuf::from(env_path);
            let _ = fs::create_dir_all(&p);
            return p;
        }

        let root = app_root_dir();

        // 2. Portable mode check
        let portable_marker = root.join("portable");
        let local_data = root.join("data");
        if portable_marker.exists() || local_data.is_dir() {
            let _ = fs::create_dir_all(&local_data);
            return local_data;
        }

        // 3. System standard data directory
        let standard_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("murasame");

        let _ = fs::create_dir_all(&standard_dir);
        standard_dir
    })
}

/// Returns the path to the chat SQLite database.
pub fn database_path() -> PathBuf {
    let dir = data_dir();
    let _ = fs::create_dir_all(dir);
    dir.join("chat.db")
}

/// Returns the path to the logs directory.
pub fn logs_dir() -> PathBuf {
    let p = data_dir().join("logs");
    let _ = fs::create_dir_all(&p);
    p
}

/// Returns the path to the crash log file.
pub fn crash_log_path() -> PathBuf {
    logs_dir().join("crash.log")
}

/// Returns the path to the global configuration file `config.toml`.
/// If `config.toml` exists in the application root, it is preferred (development / portable).
/// Otherwise, `config.toml` in `data_dir` is used.
pub fn config_path() -> PathBuf {
    let local = app_root_dir().join("config.toml");
    if local.exists() {
        local
    } else {
        data_dir().join("config.toml")
    }
}

/// Loads key-value pairs from `.env` in `app_root_dir()` into the process environment
/// if the variable is not already defined.
pub fn load_dotenv() {
    let p = app_root_dir().join(".env");
    if p.exists() {
        if let Ok(content) = fs::read_to_string(&p) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = trimmed.split_once('=') {
                    let key = k.trim();
                    let val = v.trim().trim_matches('"').trim_matches('\'');
                    if std::env::var(key).is_err() {
                        unsafe {
                            std::env::set_var(key, val);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_and_logs_paths() {
        let db_p = database_path();
        let log_p = crash_log_path();

        assert_eq!(db_p.file_name().unwrap(), "chat.db");
        assert_eq!(log_p.file_name().unwrap(), "crash.log");

        let data = data_dir();
        assert!(db_p.starts_with(data));
        assert!(log_p.starts_with(data));
    }

    #[test]
    fn test_resolve_resource_paths() {
        // Murasame model should resolve to an existing path under app_root
        let live2d = resolve_resource_path("Murasame_live2d/Murasame.model3.json");
        assert!(live2d.exists(), "Live2D model should exist at {}", live2d.display());

        // Beatrice submodule model should resolve to existing path under models/
        let beatrice = resolve_resource_path("models/murasame-beatrice-v1");
        assert!(beatrice.exists(), "Beatrice model submodule should exist at {}", beatrice.display());
    }
}

