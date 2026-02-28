use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use wait_timeout::ChildExt;

use crate::config::{Limits, Program};

#[derive(Debug)]
pub struct RunOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

#[derive(Debug, Clone)]
struct MountSpec {
    host: String,
    container: String,
    mode: Option<String>,
}

enum RunMode {
    Local,
    Docker,
}

pub fn run_program(
    program: &Program,
    input: &str,
    config_dir: &Path,
    timeout_ms: u64,
    limits: &Limits,
) -> Result<RunOutput> {
    let mounts = parse_mounts(&program.mounts)?;

    if let Some(image) = &program.image {
        let docker_cmd = build_docker_cmd(image, &program.cmd, &mounts, config_dir, limits)?;
        return run_command(
            &docker_cmd,
            input,
            config_dir,
            timeout_ms,
            limits,
            RunMode::Docker,
        );
    }

    if program.cmd.is_empty() {
        bail!("program cmd is empty");
    }

    let local_cmd = resolve_local_cmd(&program.cmd, &mounts, config_dir)?;
    run_command(
        &local_cmd,
        input,
        config_dir,
        timeout_ms,
        limits,
        RunMode::Local,
    )
}

fn run_command(
    command_argv: &[String],
    input: &str,
    config_dir: &Path,
    timeout_ms: u64,
    limits: &Limits,
    mode: RunMode,
) -> Result<RunOutput> {
    if command_argv.is_empty() {
        bail!("empty command");
    }

    let mut command = Command::new(&command_argv[0]);
    command
        .args(&command_argv[1..])
        .current_dir(config_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    if matches!(mode, RunMode::Local) {
        let limits = limits.clone();
        unsafe {
            command.pre_exec(move || apply_limits(&limits));
        }
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn command: {}", command_argv.join(" ")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .context("failed to write stdin")?;
    }

    let stdout = child.stdout.take().context("failed to capture stdout")?;
    let stderr = child.stderr.take().context("failed to capture stderr")?;

    let stdout_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::BufReader::new(stdout).read_to_end(&mut buf);
        buf
    });

    let stderr_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::BufReader::new(stderr).read_to_end(&mut buf);
        buf
    });

    let duration = Duration::from_millis(timeout_ms);
    let mut timed_out = false;

    let status = match child.wait_timeout(duration)? {
        Some(status) => status,
        None => {
            timed_out = true;
            let _ = child.kill();
            child.wait().context("failed waiting killed process")?
        }
    };

    let stdout = stdout_handle
        .join()
        .map_err(|_| anyhow::anyhow!("stdout thread panicked"))?;
    let stderr = stderr_handle
        .join()
        .map_err(|_| anyhow::anyhow!("stderr thread panicked"))?;

    Ok(RunOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout).to_string(),
        stderr: String::from_utf8_lossy(&stderr).to_string(),
        timed_out,
    })
}

fn build_docker_cmd(
    image: &str,
    cmd: &[String],
    mounts: &[MountSpec],
    config_dir: &Path,
    limits: &Limits,
) -> Result<Vec<String>> {
    let mut argv = vec![
        "docker".to_string(),
        "run".to_string(),
        "--rm".to_string(),
        "-i".to_string(),
        "--network".to_string(),
        "none".to_string(),
    ];

    if let Some(memory_mb) = limits.memory_mb {
        argv.push("--memory".to_string());
        argv.push(format!("{memory_mb}m"));
    }

    if let Some(nproc) = limits.nproc {
        argv.push("--pids-limit".to_string());
        argv.push(nproc.to_string());
    }

    for mount in mounts {
        argv.push("-v".to_string());
        argv.push(render_docker_mount(mount, config_dir)?);
    }

    argv.push(image.to_string());
    argv.extend(cmd.iter().cloned());

    Ok(argv)
}

fn render_docker_mount(mount: &MountSpec, config_dir: &Path) -> Result<String> {
    let host_path = host_path_from_mount(&mount.host, config_dir);
    let host_path = fs::canonicalize(&host_path).unwrap_or(host_path);

    let mut rendered = format!(
        "{}:{}",
        host_path.to_string_lossy(),
        mount.container.as_str()
    );

    if let Some(mode) = &mount.mode {
        rendered.push(':');
        rendered.push_str(mode);
    }

    Ok(rendered)
}

fn resolve_local_cmd(
    cmd: &[String],
    mounts: &[MountSpec],
    config_dir: &Path,
) -> Result<Vec<String>> {
    let mount_map = build_local_mount_map(mounts, config_dir);
    Ok(cmd
        .iter()
        .map(|arg| mount_map.get(arg).cloned().unwrap_or_else(|| arg.clone()))
        .collect())
}

fn build_local_mount_map(mounts: &[MountSpec], config_dir: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for mount in mounts {
        let host_path = host_path_from_mount(&mount.host, config_dir);
        let host_path = fs::canonicalize(&host_path).unwrap_or(host_path);

        map.insert(
            mount.container.clone(),
            host_path.to_string_lossy().to_string(),
        );
    }

    map
}

fn host_path_from_mount(host: &str, config_dir: &Path) -> PathBuf {
    if Path::new(host).is_absolute() {
        PathBuf::from(host)
    } else {
        config_dir.join(host)
    }
}

fn parse_mounts(mounts: &[String]) -> Result<Vec<MountSpec>> {
    let mut parsed = Vec::with_capacity(mounts.len());

    for mount in mounts {
        let mut parts = mount.split(':');
        let host = parts.next().unwrap_or_default().trim();
        let container = parts.next().unwrap_or_default().trim();
        let mode = parts.next().map(str::trim);

        if host.is_empty() || container.is_empty() {
            bail!("invalid mount syntax: {mount}. expected host:container[:mode]");
        }

        if parts.next().is_some() {
            bail!("invalid mount syntax: {mount}. too many ':' separators");
        }

        let mode = mode.filter(|m| !m.is_empty()).map(str::to_string);

        parsed.push(MountSpec {
            host: host.to_string(),
            container: container.to_string(),
            mode,
        });
    }

    Ok(parsed)
}

#[cfg(unix)]
fn apply_limits(limits: &Limits) -> std::io::Result<()> {
    if let Some(cpu_seconds) = limits.cpu_seconds {
        set_rlimit(libc::RLIMIT_CPU, cpu_seconds as libc::rlim_t)?;
    }
    if let Some(memory_mb) = limits.memory_mb {
        let bytes = (memory_mb * 1024 * 1024) as libc::rlim_t;
        set_rlimit(libc::RLIMIT_AS, bytes)?;
    }
    if let Some(file_size_kb) = limits.file_size_kb {
        let bytes = (file_size_kb * 1024) as libc::rlim_t;
        set_rlimit(libc::RLIMIT_FSIZE, bytes)?;
    }
    if let Some(nofile) = limits.nofile {
        set_rlimit(libc::RLIMIT_NOFILE, nofile as libc::rlim_t)?;
    }
    if let Some(nproc) = limits.nproc {
        set_rlimit(libc::RLIMIT_NPROC, nproc as libc::rlim_t)?;
    }

    Ok(())
}

#[cfg(unix)]
fn set_rlimit(resource: libc::c_int, value: libc::rlim_t) -> std::io::Result<()> {
    let lim = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };

    let code = unsafe { libc::setrlimit(resource as _, &lim) };
    if code == 0 {
        return Ok(());
    }

    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EINVAL) {
        // Some platforms do not support every RLIMIT resource.
        return Ok(());
    }

    Err(err)
}
