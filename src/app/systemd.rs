use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

const DEFAULT_SERVICE_NAME: &str = "slotstrike";
const DEFAULT_SYSTEMD_DIR: &str = "/etc/systemd/system";
const DEFAULT_CONFIG_PATH: &str = "slotstrike.toml";

#[derive(Clone, Debug, Eq, PartialEq)]
struct ServiceOptions {
    service_name: String,
    service_user: String,
    service_group: String,
    systemd_dir: PathBuf,
    config_path: PathBuf,
    working_dir: PathBuf,
    bin_path: PathBuf,
    enable_now: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServiceAction {
    Install,
    Uninstall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameField {
    Name,
    User,
    Group,
}

impl NameField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Name => "service name",
            Self::User => "service user",
            Self::Group => "service group",
        }
    }
}

impl std::fmt::Display for NameField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathField {
    SystemdDir,
    ConfigPath,
    WorkingDir,
    BinaryPath,
}

impl PathField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::SystemdDir => "systemd dir",
            Self::ConfigPath => "config path",
            Self::WorkingDir => "working dir",
            Self::BinaryPath => "binary path",
        }
    }
}

impl std::fmt::Display for PathField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemctlAction {
    DaemonReload,
    EnableNow,
    DisableNow,
    ResetFailed,
}

impl SystemctlAction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DaemonReload => "daemon-reload",
            Self::EnableNow => "enable --now",
            Self::DisableNow => "disable --now",
            Self::ResetFailed => "reset-failed",
        }
    }
}

impl std::fmt::Display for SystemctlAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum SystemdError {
    #[error("use only one of --install-service or --uninstall-service")]
    ConflictingServiceActions,
    #[error(transparent)]
    ServiceOptions(#[from] ServiceOptionsError),
    #[error(transparent)]
    ServiceInstall(#[from] ServiceInstallError),
    #[error(transparent)]
    ServiceUninstall(#[from] ServiceUninstallError),
}

#[derive(Debug, Error)]
pub enum ServiceOptionsError {
    #[error("{field} must not be empty")]
    EmptyName { field: NameField },
    #[error("{field} must not contain whitespace or '/'")]
    InvalidNameCharacters { field: NameField },
    #[error("failed to resolve absolute path for {field}")]
    ResolveAbsolutePath {
        field: PathField,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to resolve current dir")]
    ResolveCurrentDir {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to resolve current binary path")]
    ResolveCurrentBinaryPath {
        #[source]
        source: std::io::Error,
    },
    #[error("{field} must not contain spaces for systemd compatibility")]
    PathContainsSpaces { field: PathField },
}

#[derive(Debug, Error)]
pub enum ServiceInstallError {
    #[error("binary not found at {path}")]
    BinaryNotFound { path: PathBuf },
    #[error("config not found at {path}")]
    ConfigNotFound { path: PathBuf },
    #[error("failed to create systemd dir at {path}")]
    CreateSystemdDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write unit file at {path}")]
    WriteUnitFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Systemctl(#[from] SystemctlError),
}

#[derive(Debug, Error)]
pub enum ServiceUninstallError {
    #[error("failed to remove unit file at {path}")]
    RemoveUnitFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Systemctl(#[from] SystemctlError),
}

#[derive(Debug, Error)]
pub enum SystemctlError {
    #[error("failed to execute systemctl {action}")]
    Execute {
        action: SystemctlAction,
        #[source]
        source: std::io::Error,
    },
    #[error("systemctl {action} failed with exit code {code:?}")]
    Failed {
        action: SystemctlAction,
        code: Option<i32>,
    },
}

pub fn maybe_handle_service_command(args: &[String]) -> Result<bool, SystemdError> {
    let install = arg_flag(args, "--install-service");
    let uninstall = arg_flag(args, "--uninstall-service");

    if !install && !uninstall {
        return Ok(false);
    }

    if install && uninstall {
        return Err(SystemdError::ConflictingServiceActions);
    }

    let action = if install {
        ServiceAction::Install
    } else {
        ServiceAction::Uninstall
    };
    let options = build_options(args)?;

    match action {
        ServiceAction::Install => install_service(&options)?,
        ServiceAction::Uninstall => uninstall_service(&options)?,
    }

    Ok(true)
}

fn build_options(args: &[String]) -> Result<ServiceOptions, ServiceOptionsError> {
    let service_name =
        arg_value(args, "--service-name").unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_owned());
    validate_name(&service_name, NameField::Name)?;

    let service_user = arg_value(args, "--service-user")
        .or_else(|| env::var("SUDO_USER").ok())
        .or_else(|| env::var("USER").ok())
        .unwrap_or_else(|| "root".to_owned());
    validate_name(&service_user, NameField::User)?;

    let service_group = arg_value(args, "--service-group")
        .or_else(|| primary_group_for_user(&service_user))
        .unwrap_or_else(|| service_user.clone());
    validate_name(&service_group, NameField::Group)?;

    let systemd_dir = absolutize(
        arg_value(args, "--systemd-dir").unwrap_or_else(|| DEFAULT_SYSTEMD_DIR.to_owned()),
        PathField::SystemdDir,
    )?;
    let config_path = absolutize(
        arg_value(args, "--config").unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_owned()),
        PathField::ConfigPath,
    )?;
    let working_dir =
        env::current_dir().map_err(|source| ServiceOptionsError::ResolveCurrentDir { source })?;
    let bin_path = env::current_exe()
        .map_err(|source| ServiceOptionsError::ResolveCurrentBinaryPath { source })?;

    ensure_no_spaces(&systemd_dir, PathField::SystemdDir)?;
    ensure_no_spaces(&config_path, PathField::ConfigPath)?;
    ensure_no_spaces(&working_dir, PathField::WorkingDir)?;
    ensure_no_spaces(&bin_path, PathField::BinaryPath)?;

    Ok(ServiceOptions {
        service_name,
        service_user,
        service_group,
        systemd_dir,
        config_path,
        working_dir,
        bin_path,
        enable_now: !arg_flag(args, "--no-enable"),
    })
}

fn install_service(options: &ServiceOptions) -> Result<(), ServiceInstallError> {
    if !options.bin_path.is_file() {
        return Err(ServiceInstallError::BinaryNotFound {
            path: options.bin_path.clone(),
        });
    }

    if !options.config_path.is_file() {
        return Err(ServiceInstallError::ConfigNotFound {
            path: options.config_path.clone(),
        });
    }

    fs::create_dir_all(&options.systemd_dir).map_err(|source| {
        ServiceInstallError::CreateSystemdDir {
            path: options.systemd_dir.clone(),
            source,
        }
    })?;

    let unit_file_name = format!("{}.service", options.service_name);
    let unit_file_path = options.systemd_dir.join(&unit_file_name);
    let log_dir = options.working_dir.join("log");

    let unit_contents = render_unit(options, &log_dir);
    fs::write(&unit_file_path, unit_contents).map_err(|source| {
        ServiceInstallError::WriteUnitFile {
            path: unit_file_path.clone(),
            source,
        }
    })?;

    run_systemctl(&["daemon-reload"], SystemctlAction::DaemonReload)?;

    if options.enable_now {
        run_systemctl(
            &["enable", "--now", &unit_file_name],
            SystemctlAction::EnableNow,
        )?;
        println!(
            "Installed and started {} using config {}",
            unit_file_name,
            options.config_path.display()
        );
    } else {
        println!(
            "Installed {} (not enabled/started) using config {}",
            unit_file_name,
            options.config_path.display()
        );
    }

    Ok(())
}

fn uninstall_service(options: &ServiceOptions) -> Result<(), ServiceUninstallError> {
    let unit_file_name = format!("{}.service", options.service_name);
    let unit_file_path = options.systemd_dir.join(&unit_file_name);

    let _disable_result = run_systemctl(
        &["disable", "--now", &unit_file_name],
        SystemctlAction::DisableNow,
    );

    if unit_file_path.exists() {
        fs::remove_file(&unit_file_path).map_err(|source| {
            ServiceUninstallError::RemoveUnitFile {
                path: unit_file_path.clone(),
                source,
            }
        })?;
    }

    run_systemctl(&["daemon-reload"], SystemctlAction::DaemonReload)?;
    let _reset_failed = run_systemctl(
        &["reset-failed", &unit_file_name],
        SystemctlAction::ResetFailed,
    );

    println!("Uninstalled {}", unit_file_name);
    Ok(())
}

fn run_systemctl(args: &[&str], action: SystemctlAction) -> Result<(), SystemctlError> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|source| SystemctlError::Execute { action, source })?;

    if output.status.success() {
        return Ok(());
    }

    Err(SystemctlError::Failed {
        action,
        code: output.status.code(),
    })
}

fn primary_group_for_user(user: &str) -> Option<String> {
    let output = Command::new("id").args(["-gn", user]).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let group = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if group.is_empty() { None } else { Some(group) }
}

fn render_unit(options: &ServiceOptions, log_dir: &Path) -> String {
    format!(
        "[Unit]
Description=Slotstrike service
After=network.target

[Service]
User={}
Group={}
WorkingDirectory={}
Type=simple
ExecStart={} --config {}
ExecStartPre=/bin/mkdir -p {}
ExecStartPre=/bin/chown {}:{} {}
Restart=on-failure
RestartSec=5s
StartLimitIntervalSec=0
StartLimitBurst=0

[Install]
WantedBy=multi-user.target
",
        options.service_user,
        options.service_group,
        options.working_dir.display(),
        options.bin_path.display(),
        options.config_path.display(),
        log_dir.display(),
        options.service_user,
        options.service_group,
        log_dir.display(),
    )
}

fn arg_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index.saturating_add(1)))
        .cloned()
}

fn absolutize(value: String, field: PathField) -> Result<PathBuf, ServiceOptionsError> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return Ok(path);
    }

    env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|source| ServiceOptionsError::ResolveAbsolutePath { field, source })
}

fn validate_name(value: &str, field: NameField) -> Result<(), ServiceOptionsError> {
    if value.trim().is_empty() {
        return Err(ServiceOptionsError::EmptyName { field });
    }
    if value.contains(char::is_whitespace) || value.contains('/') {
        return Err(ServiceOptionsError::InvalidNameCharacters { field });
    }
    Ok(())
}

fn ensure_no_spaces(path: &Path, field: PathField) -> Result<(), ServiceOptionsError> {
    if path.to_string_lossy().contains(' ') {
        return Err(ServiceOptionsError::PathContainsSpaces { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_SERVICE_NAME, arg_value, maybe_handle_service_command, render_unit};
    use std::path::PathBuf;

    #[test]
    fn arg_value_reads_config_flag() {
        let args = vec![
            "--install-service".to_owned(),
            "--config".to_owned(),
            "/tmp/slotstrike.toml".to_owned(),
        ];

        assert_eq!(
            arg_value(&args, "--config"),
            Some("/tmp/slotstrike.toml".to_owned())
        );
    }

    #[test]
    fn service_flags_are_mutually_exclusive() {
        let args = vec![
            "--install-service".to_owned(),
            "--uninstall-service".to_owned(),
        ];

        let result = maybe_handle_service_command(&args);
        assert!(result.is_err());
    }

    #[test]
    fn unit_template_contains_config_arg() {
        let options = super::ServiceOptions {
            service_name: DEFAULT_SERVICE_NAME.to_owned(),
            service_user: "slotstrike".to_owned(),
            service_group: "slotstrike".to_owned(),
            systemd_dir: PathBuf::from("/etc/systemd/system"),
            config_path: PathBuf::from("/home/slotstrike/slotstrike.toml"),
            working_dir: PathBuf::from("/home/slotstrike"),
            bin_path: PathBuf::from("/usr/local/bin/slotstrike"),
            enable_now: true,
        };

        let rendered = render_unit(&options, &PathBuf::from("/home/slotstrike/log"));
        assert!(rendered.contains(
            "ExecStart=/usr/local/bin/slotstrike --config /home/slotstrike/slotstrike.toml"
        ));
    }
}
