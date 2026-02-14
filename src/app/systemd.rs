use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const DEFAULT_SERVICE_NAME: &str = "sniper";
const DEFAULT_SYSTEMD_DIR: &str = "/etc/systemd/system";
const DEFAULT_CONFIG_PATH: &str = "sniper.toml";

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

pub fn maybe_handle_service_command(args: &[String]) -> Result<bool, String> {
    let install = arg_flag(args, "--install-service");
    let uninstall = arg_flag(args, "--uninstall-service");

    if !install && !uninstall {
        return Ok(false);
    }

    if install && uninstall {
        return Err("Use only one of --install-service or --uninstall-service".to_owned());
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

fn build_options(args: &[String]) -> Result<ServiceOptions, String> {
    let service_name =
        arg_value(args, "--service-name").unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_owned());
    validate_name(&service_name, "service name")?;

    let service_user = arg_value(args, "--service-user")
        .or_else(|| env::var("SUDO_USER").ok())
        .or_else(|| env::var("USER").ok())
        .unwrap_or_else(|| "root".to_owned());
    validate_name(&service_user, "service user")?;

    let service_group = arg_value(args, "--service-group")
        .or_else(|| primary_group_for_user(&service_user))
        .unwrap_or_else(|| service_user.clone());
    validate_name(&service_group, "service group")?;

    let systemd_dir = absolutize(
        arg_value(args, "--systemd-dir").unwrap_or_else(|| DEFAULT_SYSTEMD_DIR.to_owned()),
    )?;
    let config_path =
        absolutize(arg_value(args, "--config").unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_owned()))?;
    let working_dir =
        env::current_dir().map_err(|error| format!("Failed to resolve current dir: {}", error))?;
    let bin_path = env::current_exe()
        .map_err(|error| format!("Failed to resolve current binary path: {}", error))?;

    ensure_no_spaces(&systemd_dir, "systemd dir")?;
    ensure_no_spaces(&config_path, "config path")?;
    ensure_no_spaces(&working_dir, "working dir")?;
    ensure_no_spaces(&bin_path, "binary path")?;

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

fn install_service(options: &ServiceOptions) -> Result<(), String> {
    if !options.bin_path.is_file() {
        return Err(format!("Binary not found: {}", options.bin_path.display()));
    }

    if !options.config_path.is_file() {
        return Err(format!(
            "Config not found: {}",
            options.config_path.display()
        ));
    }

    fs::create_dir_all(&options.systemd_dir).map_err(|error| {
        format!(
            "Failed to create systemd dir '{}': {}",
            options.systemd_dir.display(),
            error
        )
    })?;

    let unit_file_name = format!("{}.service", options.service_name);
    let unit_file_path = options.systemd_dir.join(&unit_file_name);
    let log_dir = options.working_dir.join("log");

    let unit_contents = render_unit(options, &log_dir);
    fs::write(&unit_file_path, unit_contents).map_err(|error| {
        format!(
            "Failed to write unit file '{}': {}",
            unit_file_path.display(),
            error
        )
    })?;

    run_systemctl(&["daemon-reload"])?;

    if options.enable_now {
        run_systemctl(&["enable", "--now", &unit_file_name])?;
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

fn uninstall_service(options: &ServiceOptions) -> Result<(), String> {
    let unit_file_name = format!("{}.service", options.service_name);
    let unit_file_path = options.systemd_dir.join(&unit_file_name);

    let _disable_result = run_systemctl(&["disable", "--now", &unit_file_name]);

    if unit_file_path.exists() {
        fs::remove_file(&unit_file_path).map_err(|error| {
            format!(
                "Failed to remove unit file '{}': {}",
                unit_file_path.display(),
                error
            )
        })?;
    }

    run_systemctl(&["daemon-reload"])?;
    let _reset_failed = run_systemctl(&["reset-failed", &unit_file_name]);

    println!("Uninstalled {}", unit_file_name);
    Ok(())
}

fn run_systemctl(args: &[&str]) -> Result<(), String> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|error| format!("Failed to execute systemctl {:?}: {}", args, error))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "systemctl {:?} failed (code {:?}). stdout='{}' stderr='{}'",
        args,
        output.status.code(),
        stdout.trim(),
        stderr.trim()
    ))
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
Description=Sniper service
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

fn absolutize(value: String) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return Ok(path);
    }

    env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|error| format!("Failed to resolve absolute path: {}", error))
}

fn validate_name(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{} must not be empty", field));
    }
    if value.contains(char::is_whitespace) || value.contains('/') {
        return Err(format!(
            "{} must not contain whitespace or '/' (got '{}')",
            field, value
        ));
    }
    Ok(())
}

fn ensure_no_spaces(path: &Path, field: &str) -> Result<(), String> {
    if path.to_string_lossy().contains(' ') {
        return Err(format!(
            "{} must not contain spaces for systemd compatibility: {}",
            field,
            path.display()
        ));
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
            "/tmp/sniper.toml".to_owned(),
        ];

        assert_eq!(
            arg_value(&args, "--config"),
            Some("/tmp/sniper.toml".to_owned())
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
            service_user: "sniper".to_owned(),
            service_group: "sniper".to_owned(),
            systemd_dir: PathBuf::from("/etc/systemd/system"),
            config_path: PathBuf::from("/home/sniper/sniper/sniper.toml"),
            working_dir: PathBuf::from("/home/sniper/sniper"),
            bin_path: PathBuf::from("/home/sniper/sniper/target/release/sniper"),
            enable_now: true,
        };
        let log_dir = PathBuf::from("/home/sniper/sniper/log");

        let rendered = render_unit(&options, &log_dir);
        assert!(rendered.contains(
            "ExecStart=/home/sniper/sniper/target/release/sniper --config /home/sniper/sniper/sniper.toml"
        ));
    }
}
