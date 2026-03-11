//! Repository maintenance helpers.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct UdsSpec {
    services: Vec<NamedByte>,
    negative_responses: Vec<NamedByte>,
}

#[derive(Debug, Deserialize)]
struct ObdSpec {
    mode1: Vec<NamedUnitByte>,
    mode9: Vec<NamedUnitByte>,
}

#[derive(Debug, Deserialize)]
struct J1939Spec {
    pgns: Vec<NamedPgn>,
}

#[derive(Debug, Deserialize)]
struct NamedByte {
    sid: Option<u8>,
    code: Option<u8>,
    name: String,
}

#[derive(Debug, Deserialize)]
struct NamedUnitByte {
    pid: u8,
    name: String,
    unit: String,
}

#[derive(Debug, Deserialize)]
struct NamedPgn {
    pgn: u32,
    name: String,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn write_if_changed(path: &Path, content: &str) -> Result<()> {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }
    fs::write(path, content)?;
    Ok(())
}

fn command_string(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_command_in(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .current_dir(root)
        .args(args)
        .status()
        .with_context(|| format!("failed to start {}", command_string(program, args)))?;
    if status.success() {
        Ok(())
    } else {
        bail!("command failed: {}", command_string(program, args))
    }
}

fn command_output_in(root: &Path, program: &str, args: &[&str]) -> Result<Output> {
    Command::new(program)
        .current_dir(root)
        .args(args)
        .output()
        .with_context(|| format!("failed to start {}", command_string(program, args)))
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn orb_repo_script(root: &Path, command: &str) -> String {
    format!(
        "if [ -f \"$HOME/.cargo/env\" ]; then . \"$HOME/.cargo/env\"; fi; cd {} && {}",
        shell_quote(root),
        command
    )
}

fn orb_args<'a>(user: Option<&'a str>, script: &'a str) -> Vec<&'a str> {
    let mut args = vec!["-m", "debian"];
    if let Some(user) = user {
        args.push("-u");
        args.push(user);
    }
    args.push("sh");
    args.push("-lc");
    args.push(script);
    args
}

fn run_orb_script(root: &Path, user: Option<&str>, script: &str) -> Result<()> {
    let wrapped = orb_repo_script(root, script);
    let args = orb_args(user, &wrapped);
    run_command_in(root, "orb", &args)
}

fn orb_output(root: &Path, user: Option<&str>, script: &str) -> Result<Output> {
    let wrapped = orb_repo_script(root, script);
    let args = orb_args(user, &wrapped);
    command_output_in(root, "orb", &args)
}

fn ensure_orb(root: &Path) -> Result<()> {
    let output = command_output_in(root, "orb", &["--help"])
        .context("OrbStack CLI is required for local Linux validation")?;
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "OrbStack CLI is installed but unavailable:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    }
}

fn ensure_orb_machine(root: &Path) -> Result<()> {
    let output = orb_output(root, None, "true")
        .context("OrbStack machine `debian` is required for local Linux validation")?;
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "OrbStack machine `debian` is unavailable:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    }
}

fn ensure_orb_cargo_deny(root: &Path) -> Result<()> {
    let output = orb_output(root, None, "cargo deny --version")
        .context("failed to probe cargo-deny in OrbStack Debian")?;
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "cargo-deny is required inside OrbStack Debian for `ci-linux-local`.\nInstall it with:\norb -m debian sh -lc 'if [ -f \"$HOME/.cargo/env\" ]; then . \"$HOME/.cargo/env\"; fi; cargo install cargo-deny --locked'\n\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    }
}

fn provision_vcan(root: &Path) -> Result<()> {
    run_orb_script(
        root,
        Some("root"),
        "modprobe vcan || true; ip link add dev vcan0 type vcan || true; ip link set up vcan0",
    )
    .context("failed to provision vcan0 in OrbStack Debian")
}

fn run_socketcan_smoke_test(root: &Path) -> Result<()> {
    let dump_script = orb_repo_script(
        root,
        "timeout 20s target/debug/cantools dump --interface vcan0 --count 1",
    );
    let dump = Command::new("orb")
        .current_dir(root)
        .args(["-m", "debian", "sh", "-lc", &dump_script])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start SocketCAN smoke-test receiver in OrbStack Debian")?;

    // Give the receiver a moment to bind before the sender injects traffic.
    thread::sleep(Duration::from_secs(1));

    let send_output = orb_output(
        root,
        None,
        "target/debug/cantools send --interface vcan0 --id 123 --data 01020304",
    )
    .context("failed to send SocketCAN smoke-test frame in OrbStack Debian")?;
    if !send_output.status.success() {
        bail!(
            "SocketCAN smoke-test sender failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&send_output.stdout),
            String::from_utf8_lossy(&send_output.stderr)
        );
    }

    let dump_output = dump
        .wait_with_output()
        .context("failed waiting for SocketCAN smoke-test receiver")?;
    if !dump_output.status.success() {
        bail!(
            "SocketCAN smoke-test receiver failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&dump_output.stdout),
            String::from_utf8_lossy(&dump_output.stderr)
        );
    }

    let receiver_stdout = String::from_utf8_lossy(&dump_output.stdout);
    if !receiver_stdout.contains("123 [ 4] 01 02 03 04") {
        bail!(
            "SocketCAN smoke-test receiver did not report the injected frame:\n{}",
            receiver_stdout
        );
    }

    Ok(())
}

fn ci_hosted(root: &Path) -> Result<()> {
    run_command_in(root, "cargo", &["fmt", "--check"])?;
    run_command_in(
        root,
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run_command_in(root, "cargo", &["test", "--workspace"])?;
    Ok(())
}

fn ci_linux_local(root: &Path) -> Result<()> {
    ensure_orb(root)?;
    ensure_orb_machine(root)?;
    ensure_orb_cargo_deny(root)?;
    provision_vcan(root)?;
    run_orb_script(root, None, "cargo build --workspace")?;
    run_orb_script(root, None, "cargo test --workspace")?;
    run_orb_script(root, None, "cargo deny check")?;
    run_socketcan_smoke_test(root)?;
    Ok(())
}

fn ci_all_local(root: &Path) -> Result<()> {
    ci_hosted(root)?;
    ci_linux_local(root)?;
    Ok(())
}

fn install_hooks(root: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let hook_path = root.join(".githooks/pre-push");
        let mut permissions = fs::metadata(&hook_path)
            .with_context(|| format!("missing hook {}", hook_path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&hook_path, permissions)
            .with_context(|| format!("failed to update mode for {}", hook_path.display()))?;
    }

    run_command_in(root, "git", &["config", "core.hooksPath", ".githooks"])?;
    let output = command_output_in(root, "git", &["config", "--get", "core.hooksPath"])?;
    if !output.status.success() {
        bail!("failed to read core.hooksPath after installation");
    }
    println!(
        "Configured core.hooksPath={}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
    Ok(())
}

fn codegen(root: &Path) -> Result<()> {
    let uds: UdsSpec = toml::from_str(&fs::read_to_string(
        root.join("spec-data/uds/services.toml"),
    )?)?;
    let obd: ObdSpec = toml::from_str(&fs::read_to_string(root.join("spec-data/obd/pids.toml"))?)?;
    let j1939: J1939Spec =
        toml::from_str(&fs::read_to_string(root.join("spec-data/j1939/pgns.toml"))?)?;

    let uds_generated = format!(
        "//! Generated UDS tables. Regenerate with `cargo run -p xtask -- codegen`.\n\n\
/// Generated UDS service metadata.\n\
#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
pub struct GeneratedService {{\n\
    /// ISO 14229 service identifier.\n\
    pub sid: u8,\n\
    /// Human-readable service name.\n\
    pub name: &'static str,\n\
}}\n\n\
/// Generated UDS negative response metadata.\n\
#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
pub struct GeneratedNegativeResponse {{\n\
    /// ISO 14229 negative response code.\n\
    pub code: u8,\n\
    /// Human-readable negative response name.\n\
    pub name: &'static str,\n\
}}\n\n\
/// Generated UDS services keyed by service identifier.\n\
pub const GENERATED_SERVICES: &[GeneratedService] = &[\n{}\n];\n\n\
/// Generated UDS negative responses keyed by response code.\n\
pub const GENERATED_NEGATIVE_RESPONSES: &[GeneratedNegativeResponse] = &[\n{}\n];\n",
        uds.services
            .iter()
            .map(|entry| format!(
                "    GeneratedService {{ sid: 0x{:02X}, name: {:?} }},",
                entry.sid.expect("sid"),
                entry.name
            ))
            .collect::<Vec<_>>()
            .join("\n"),
        uds.negative_responses
            .iter()
            .map(|entry| format!(
                "    GeneratedNegativeResponse {{ code: 0x{:02X}, name: {:?} }},",
                entry.code.expect("code"),
                entry.name
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
    write_if_changed(
        &root.join("crates/cantools-uds/src/generated.rs"),
        &uds_generated,
    )?;

    let obd_generated = format!(
        "//! Generated OBD tables. Regenerate with `cargo run -p xtask -- codegen`.\n\n\
/// Generated OBD PID metadata.\n\
#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
pub struct GeneratedPid {{\n\
    /// OBD service mode.\n\
    pub mode: u8,\n\
    /// OBD parameter identifier.\n\
    pub pid: u8,\n\
    /// Human-readable PID name.\n\
    pub name: &'static str,\n\
    /// Engineering unit for decoded values.\n\
    pub unit: &'static str,\n\
}}\n\n\
/// Generated OBD PIDs keyed by service mode and PID.\n\
pub const GENERATED_PIDS: &[GeneratedPid] = &[\n{}\n];\n",
        obd.mode1
            .iter()
            .map(|entry| format!(
                "    GeneratedPid {{ mode: 0x01, pid: 0x{:02X}, name: {:?}, unit: {:?} }},",
                entry.pid, entry.name, entry.unit
            ))
            .chain(obd.mode9.iter().map(|entry| {
                format!(
                    "    GeneratedPid {{ mode: 0x09, pid: 0x{:02X}, name: {:?}, unit: {:?} }},",
                    entry.pid, entry.name, entry.unit
                )
            }))
            .collect::<Vec<_>>()
            .join("\n")
    );
    write_if_changed(
        &root.join("crates/cantools-obd/src/generated.rs"),
        &obd_generated,
    )?;

    let j1939_generated = format!(
        "//! Generated J1939 tables. Regenerate with `cargo run -p xtask -- codegen`.\n\n\
/// Generated J1939 PGN metadata.\n\
#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
pub struct GeneratedPgn {{\n\
    /// Parameter Group Number.\n\
    pub pgn: u32,\n\
    /// Human-readable PGN name.\n\
    pub name: &'static str,\n\
}}\n\n\
/// Generated J1939 PGN definitions keyed by PGN.\n\
pub const GENERATED_PGNS: &[GeneratedPgn] = &[\n{}\n];\n",
        j1939
            .pgns
            .iter()
            .map(|entry| format!(
                "    GeneratedPgn {{ pgn: 0x{:06X}, name: {:?} }},",
                entry.pgn, entry.name
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
    write_if_changed(
        &root.join("crates/cantools-j1939/src/generated.rs"),
        &j1939_generated,
    )?;

    Ok(())
}

fn check_placeholders(root: &Path) -> Result<()> {
    let matcher = Regex::new(&format!(
        r"\btodo!\b|\bunimplemented!\b|\b{}\b",
        ["TO", "DO"].concat()
    ))?;
    let mut failures = Vec::new();
    for path in walkdir(root)? {
        if path
            .components()
            .any(|component| component.as_os_str() == "target" || component.as_os_str() == ".git")
        {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("AGENTS.md") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        if matcher.is_match(&content) {
            failures.push(path);
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!(
            "placeholder markers remain in:\n{}",
            failures
                .into_iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

fn walkdir(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn main() -> Result<()> {
    let root = repo_root();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("codegen") => codegen(&root),
        Some("check-placeholders") => check_placeholders(&root),
        Some("ci-hosted") => ci_hosted(&root),
        Some("ci-linux-local") => ci_linux_local(&root),
        Some("ci-all-local") | Some("ci") => ci_all_local(&root),
        Some("install-hooks") => install_hooks(&root),
        Some(other) => bail!("unknown xtask command {other}"),
        None => {
            eprintln!(
                "usage: cargo run -p xtask -- <codegen|check-placeholders|ci-hosted|ci-linux-local|ci-all-local|ci|install-hooks>"
            );
            Ok(())
        }
    }
}
