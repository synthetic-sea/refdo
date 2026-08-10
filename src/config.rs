use std::{
    error::Error,
    fmt,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};

const APP_DIRECTORY: &str = "refdo";
const CONFIG_FILE: &str = "config.toml";

fn config_path(base_dirs: &BaseDirs) -> PathBuf {
    #[cfg(target_os = "macos")]
    let config_root = base_dirs.home_dir().join(".config");
    #[cfg(not(target_os = "macos"))]
    let config_root = base_dirs.config_dir().to_owned();

    config_root.join(APP_DIRECTORY).join(CONFIG_FILE)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) theme: ThemeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ThemeConfig {
    pub(crate) light: String,
    pub(crate) dark: String,
    pub(crate) mode: ThemeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemeMode {
    Light,
    Dark,
    System,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: ThemeConfig::default(),
        }
    }
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            light: "tokyo-night-day".to_owned(),
            dark: "tokyo-night".to_owned(),
            mode: ThemeMode::System,
        }
    }
}

impl Config {
    pub(crate) fn load() -> Result<Self, ConfigError> {
        let base_dirs = BaseDirs::new().ok_or(ConfigError::ConfigDirectoryUnavailable)?;
        Self::load_from(&config_path(&base_dirs))
    }

    fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let parent = path.parent().ok_or_else(|| ConfigError::InvalidPath {
            path: path.to_owned(),
        })?;
        fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDirectory {
            path: parent.to_owned(),
            source,
        })?;

        match fs::read_to_string(path) {
            Ok(contents) => Self::parse(path, &contents),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Self::create_default(path),
            Err(source) => Err(ConfigError::Read {
                path: path.to_owned(),
                source,
            }),
        }
    }

    fn parse(path: &Path, contents: &str) -> Result<Self, ConfigError> {
        toml::from_str(contents).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })
    }

    fn create_default(path: &Path) -> Result<Self, ConfigError> {
        let config = Self::default();
        let contents =
            toml::to_string_pretty(&config).map_err(|source| ConfigError::Serialize {
                path: path.to_owned(),
                source,
            })?;

        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => {
                file.write_all(contents.as_bytes())
                    .map_err(|source| ConfigError::Write {
                        path: path.to_owned(),
                        source,
                    })?;
                Ok(config)
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                let contents = fs::read_to_string(path).map_err(|source| ConfigError::Read {
                    path: path.to_owned(),
                    source,
                })?;
                Self::parse(path, &contents)
            }
            Err(source) => Err(ConfigError::CreateFile {
                path: path.to_owned(),
                source,
            }),
        }
    }
}

#[derive(Debug)]
pub(crate) enum ConfigError {
    ConfigDirectoryUnavailable,
    InvalidPath {
        path: PathBuf,
    },
    CreateDirectory {
        path: PathBuf,
        source: io::Error,
    },
    Read {
        path: PathBuf,
        source: io::Error,
    },
    CreateFile {
        path: PathBuf,
        source: io::Error,
    },
    Write {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    Serialize {
        path: PathBuf,
        source: toml::ser::Error,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigDirectoryUnavailable => {
                write!(
                    formatter,
                    "could not determine the platform configuration directory"
                )
            }
            Self::InvalidPath { path } => {
                write!(
                    formatter,
                    "configuration path has no parent: {}",
                    path.display()
                )
            }
            Self::CreateDirectory { path, source } => write!(
                formatter,
                "could not create configuration directory {}: {source}",
                path.display()
            ),
            Self::Read { path, source } => write!(
                formatter,
                "could not read configuration file {}: {source}",
                path.display()
            ),
            Self::CreateFile { path, source } => write!(
                formatter,
                "could not create configuration file {}: {source}",
                path.display()
            ),
            Self::Write { path, source } => write!(
                formatter,
                "could not write configuration file {}: {source}",
                path.display()
            ),
            Self::Parse { path, source } => write!(
                formatter,
                "invalid configuration in {}: {source}",
                path.display()
            ),
            Self::Serialize { path, source } => write!(
                formatter,
                "could not serialize default configuration for {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CreateDirectory { source, .. }
            | Self::Read { source, .. }
            | Self::CreateFile { source, .. }
            | Self::Write { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Serialize { source, .. } => Some(source),
            Self::ConfigDirectoryUnavailable | Self::InvalidPath { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "refdo-config-test-{}-{sequence}",
                std::process::id()
            ));
            Self(path)
        }

        fn config_path(&self) -> PathBuf {
            self.0.join(APP_DIRECTORY).join(CONFIG_FILE)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_config(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("test config path has a parent")).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn assert_parse_error(path: &Path, error: ConfigError) {
        assert!(matches!(error, ConfigError::Parse { .. }));
        assert!(error.to_string().contains(&path.display().to_string()));
        assert!(error.source().is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_config_path_uses_home_dot_config() {
        let base_dirs = BaseDirs::new().unwrap();

        assert_eq!(
            config_path(&base_dirs),
            base_dirs
                .home_dir()
                .join(".config")
                .join(APP_DIRECTORY)
                .join(CONFIG_FILE)
        );
    }

    #[test]
    fn loads_valid_config() {
        let directory = TestDirectory::new();
        let path = directory.config_path();
        write_config(
            &path,
            "[theme]\nlight = \"tokyo-night-day\"\ndark = \"tokyo-night\"\nmode = \"dark\"\n",
        );

        let config = Config::load_from(&path).unwrap();

        assert_eq!(
            config,
            Config {
                theme: ThemeConfig {
                    light: "tokyo-night-day".to_owned(),
                    dark: "tokyo-night".to_owned(),
                    mode: ThemeMode::Dark,
                },
            }
        );
    }

    #[test]
    fn creates_a_missing_default_file_and_reloads_it() {
        let directory = TestDirectory::new();
        let path = directory.config_path();

        let created = Config::load_from(&path).unwrap();
        let emitted = fs::read_to_string(&path).unwrap();
        let reloaded = Config::load_from(&path).unwrap();

        assert_eq!(created, Config::default());
        assert_eq!(reloaded, created);
        assert!(emitted.contains("[theme]"));
        assert!(emitted.contains("light = \"tokyo-night-day\""));
        assert!(emitted.contains("dark = \"tokyo-night\""));
        assert!(emitted.contains("mode = \"system\""));
    }

    #[test]
    fn rejects_malformed_toml_with_path_and_cause() {
        let directory = TestDirectory::new();
        let path = directory.config_path();
        write_config(&path, "[theme\n");

        let error = Config::load_from(&path).unwrap_err();

        assert_parse_error(&path, error);
    }

    #[test]
    fn rejects_invalid_theme_mode_with_path_and_cause() {
        let directory = TestDirectory::new();
        let path = directory.config_path();
        write_config(
            &path,
            "[theme]\nlight = \"tokyo-night-day\"\ndark = \"tokyo-night\"\nmode = \"automatic\"\n",
        );

        let error = Config::load_from(&path).unwrap_err();

        assert_parse_error(&path, error);
    }

    #[test]
    fn rejects_unknown_keys_at_each_level() {
        for contents in [
            "extra = true\n[theme]\nlight = \"tokyo-night-day\"\ndark = \"tokyo-night\"\nmode = \"system\"\n",
            "[theme]\nlight = \"tokyo-night-day\"\ndark = \"tokyo-night\"\nmode = \"system\"\nextra = true\n",
        ] {
            let directory = TestDirectory::new();
            let path = directory.config_path();
            write_config(&path, contents);

            let error = Config::load_from(&path).unwrap_err();

            assert_parse_error(&path, error);
        }
    }
}
