use std::{
    collections::BTreeMap,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) theme: ThemeConfig,
    #[serde(default, skip_serializing_if = "DispatchSettings::is_empty")]
    pub(crate) dispatch: DispatchSettings,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) dispatches: BTreeMap<String, DispatchDefinition>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DispatchSettings {
    pub(crate) generate_branch_name_command: Option<String>,
}

impl DispatchSettings {
    fn is_empty(&self) -> bool {
        self.generate_branch_name_command.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DispatchDefinition {
    pub(crate) command: String,
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
        let config: Self = toml::from_str(contents).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
        config
            .validate()
            .map_err(|message| ConfigError::Validation {
                path: path.to_owned(),
                message,
            })?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        for (name, definition) in &self.dispatches {
            if name.is_empty() {
                return Err("dispatch name must not be empty".to_owned());
            }
            if name.chars().any(char::is_whitespace) {
                return Err(format!(
                    "dispatch name '{name}' must not contain whitespace"
                ));
            }
            if definition.command.is_empty() {
                return Err(format!("dispatch '{name}' command must not be empty"));
            }
            validate_template(&definition.command, TemplateKind::Dispatch(name))?;
        }

        if let Some(command) = &self.dispatch.generate_branch_name_command {
            if command.is_empty() {
                return Err("generate_branch_name_command must not be empty".to_owned());
            }
            validate_template(command, TemplateKind::BranchGenerator)?;
        }

        Ok(())
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

#[derive(Clone, Copy)]
enum TemplateKind<'a> {
    Dispatch(&'a str),
    BranchGenerator,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShellQuote {
    Unquoted,
    Single,
    Double,
    Backtick,
}

fn validate_template(template: &str, kind: TemplateKind<'_>) -> Result<(), String> {
    let bytes = template.as_bytes();
    let mut index = 0;
    let mut quote = ShellQuote::Unquoted;
    let mut escaped = false;

    while index < bytes.len() {
        if bytes[index..].starts_with(b"{{") {
            let end = placeholder_end(bytes, index);
            let token = &template[index..end];
            let supported = match kind {
                TemplateKind::Dispatch(_) => token == "{{CONTENT}}" || token == "{{BRANCH}}",
                TemplateKind::BranchGenerator => token == "{{CONTENT}}",
            };
            if !supported {
                return Err(invalid_placeholder(kind, token));
            }
            if quote != ShellQuote::Unquoted || escaped {
                return Err(quoted_placeholder(kind, token));
            }
            escaped = false;
            index = end;
            continue;
        }
        if bytes[index..].starts_with(b"}}") {
            return Err(invalid_placeholder(kind, "}}"));
        }

        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' && quote != ShellQuote::Single {
            escaped = true;
            index += 1;
            continue;
        }

        quote = match (quote, byte) {
            (ShellQuote::Unquoted, b'\'') => ShellQuote::Single,
            (ShellQuote::Single, b'\'') => ShellQuote::Unquoted,
            (ShellQuote::Unquoted, b'"') => ShellQuote::Double,
            (ShellQuote::Double, b'"') => ShellQuote::Unquoted,
            (ShellQuote::Unquoted, b'`') => ShellQuote::Backtick,
            (ShellQuote::Backtick, b'`') => ShellQuote::Unquoted,
            _ => quote,
        };
        index += 1;
    }

    Ok(())
}

fn placeholder_end(template: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while index + 1 < template.len() {
        if template[index..].starts_with(b"}}") {
            return index + 2;
        }
        if template[index].is_ascii_whitespace()
            || matches!(template[index], b'\'' | b'"' | b'`' | b';' | b'|' | b'&')
        {
            return index;
        }
        index += 1;
    }
    template.len()
}

fn invalid_placeholder(kind: TemplateKind<'_>, token: &str) -> String {
    match kind {
        TemplateKind::Dispatch(name) => {
            format!("dispatch '{name}' command contains invalid placeholder '{token}'")
        }
        TemplateKind::BranchGenerator => {
            format!("generate_branch_name_command contains invalid placeholder '{token}'")
        }
    }
}

fn quoted_placeholder(kind: TemplateKind<'_>, token: &str) -> String {
    match kind {
        TemplateKind::Dispatch(name) => {
            format!("dispatch '{name}' placeholder '{token}' must not be quoted")
        }
        TemplateKind::BranchGenerator => {
            format!("generate_branch_name_command placeholder '{token}' must not be quoted")
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
    Validation {
        path: PathBuf,
        message: String,
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
            Self::Validation { path, message } => write!(
                formatter,
                "invalid configuration in {}: {message}",
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
            Self::ConfigDirectoryUnavailable
            | Self::InvalidPath { .. }
            | Self::Validation { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    const THEME_CONFIG: &str = concat!(
        "[theme]\n",
        "light = \"tokyo-night-day\"\n",
        "dark = \"tokyo-night\"\n",
        "mode = \"system\"\n",
    );

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

    fn assert_validation_error(extra: &str, expected_message: &str) {
        let directory = TestDirectory::new();
        let path = directory.config_path();
        write_config(&path, &format!("{THEME_CONFIG}\n{extra}"));

        let error = Config::load_from(&path).unwrap_err();

        match &error {
            ConfigError::Validation {
                path: error_path,
                message,
            } => {
                assert_eq!(error_path, &path);
                assert_eq!(message, expected_message);
            }
            other => panic!("expected validation error, got {other:?}"),
        }
        assert_eq!(
            error.to_string(),
            format!(
                "invalid configuration in {}: {expected_message}",
                path.display()
            )
        );
        assert!(error.source().is_none());
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
                dispatch: DispatchSettings::default(),
                dispatches: BTreeMap::new(),
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
        assert!(!emitted.contains("[dispatch]"));
        assert!(!emitted.contains("[dispatches"));
    }

    #[test]
    fn loads_named_dispatches_and_branch_generator() {
        let directory = TestDirectory::new();
        let path = directory.config_path();
        let dispatch_config = concat!(
            "[dispatch]\n",
            "generate_branch_name_command = 'omp --model gemini-3.7-flash --thinking low \"Create a git branch name for the following todo item:\" {{CONTENT}}'\n",
            "\n",
            "[dispatches.implement]\n",
            "command = './scripts/implement.sh {{BRANCH}} omp {{CONTENT}}'\n",
            "\n",
            "[dispatches.report]\n",
            "command = './scripts/report.sh {{CONTENT}}'\n",
        );
        write_config(&path, &format!("{THEME_CONFIG}\n{dispatch_config}"));

        let config = Config::load_from(&path).unwrap();

        assert_eq!(
            config.dispatch.generate_branch_name_command.as_deref(),
            Some(
                "omp --model gemini-3.7-flash --thinking low \"Create a git branch name for the following todo item:\" {{CONTENT}}"
            )
        );
        assert_eq!(
            config.dispatches["implement"].command,
            "./scripts/implement.sh {{BRANCH}} omp {{CONTENT}}"
        );
        assert_eq!(
            config.dispatches["report"].command,
            "./scripts/report.sh {{CONTENT}}"
        );
    }

    #[test]
    fn rejects_invalid_dispatch_names_and_empty_commands() {
        for (extra, expected) in [
            (
                "[dispatches.\"\"]\ncommand = 'echo {{CONTENT}}'\n",
                "dispatch name must not be empty",
            ),
            (
                "[dispatches.\"two words\"]\ncommand = 'echo {{CONTENT}}'\n",
                "dispatch name 'two words' must not contain whitespace",
            ),
            (
                "[dispatches.\"two\\twords\"]\ncommand = 'echo {{CONTENT}}'\n",
                "dispatch name 'two\twords' must not contain whitespace",
            ),
            (
                "[dispatches.empty]\ncommand = ''\n",
                "dispatch 'empty' command must not be empty",
            ),
        ] {
            assert_validation_error(extra, expected);
        }
    }

    #[test]
    fn rejects_invalid_dispatch_placeholders() {
        for (command, expected) in [
            (
                "echo {{UNKNOWN}}",
                "dispatch 'run' command contains invalid placeholder '{{UNKNOWN}}'",
            ),
            (
                "echo {{CONTENT}",
                "dispatch 'run' command contains invalid placeholder '{{CONTENT}'",
            ),
            (
                "echo }}",
                "dispatch 'run' command contains invalid placeholder '}}'",
            ),
            (
                "echo '{{CONTENT}}'",
                "dispatch 'run' placeholder '{{CONTENT}}' must not be quoted",
            ),
            (
                "echo \"{{BRANCH}}\"",
                "dispatch 'run' placeholder '{{BRANCH}}' must not be quoted",
            ),
            (
                "echo `{{CONTENT}}`",
                "dispatch 'run' placeholder '{{CONTENT}}' must not be quoted",
            ),
            (
                "echo \\{{CONTENT}}",
                "dispatch 'run' placeholder '{{CONTENT}}' must not be quoted",
            ),
        ] {
            assert_validation_error(
                &format!(
                    "[dispatches.run]\ncommand = {}\n",
                    toml::Value::String(command.to_owned())
                ),
                expected,
            );
        }
    }

    #[test]
    fn rejects_invalid_branch_generators() {
        for (command, expected) in [
            ("", "generate_branch_name_command must not be empty"),
            (
                "echo {{BRANCH}}",
                "generate_branch_name_command contains invalid placeholder '{{BRANCH}}'",
            ),
            (
                "echo {{UNKNOWN}}",
                "generate_branch_name_command contains invalid placeholder '{{UNKNOWN}}'",
            ),
            (
                "echo {{CONTENT}",
                "generate_branch_name_command contains invalid placeholder '{{CONTENT}'",
            ),
            (
                "echo '{{CONTENT}}'",
                "generate_branch_name_command placeholder '{{CONTENT}}' must not be quoted",
            ),
            (
                "echo \"{{CONTENT}}\"",
                "generate_branch_name_command placeholder '{{CONTENT}}' must not be quoted",
            ),
            (
                "echo `{{CONTENT}}`",
                "generate_branch_name_command placeholder '{{CONTENT}}' must not be quoted",
            ),
            (
                "echo \\{{CONTENT}}",
                "generate_branch_name_command placeholder '{{CONTENT}}' must not be quoted",
            ),
        ] {
            assert_validation_error(
                &format!(
                    "[dispatch]\ngenerate_branch_name_command = {}\n",
                    toml::Value::String(command.to_owned())
                ),
                expected,
            );
        }
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
            "[theme]\nlight = \"tokyo-night-day\"\ndark = \"tokyo-night\"\nmode = \"system\"\n[dispatch]\nextra = true\n",
            "[theme]\nlight = \"tokyo-night-day\"\ndark = \"tokyo-night\"\nmode = \"system\"\n[dispatches.run]\ncommand = \"echo\"\nextra = true\n",
        ] {
            let directory = TestDirectory::new();
            let path = directory.config_path();
            write_config(&path, contents);

            let error = Config::load_from(&path).unwrap_err();

            assert_parse_error(&path, error);
        }
    }
}
