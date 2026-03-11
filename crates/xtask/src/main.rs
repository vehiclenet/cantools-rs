//! Repository maintenance helpers.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Result, bail};
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

fn run_command(args: &[&str]) -> Result<()> {
    let status = Command::new(args[0]).args(&args[1..]).status()?;
    if status.success() {
        Ok(())
    } else {
        bail!("command failed: {}", args.join(" "))
    }
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
        Some("ci") => {
            run_command(&["cargo", "fmt", "--check"])?;
            run_command(&["cargo", "check", "--workspace"])?;
            run_command(&["cargo", "test", "--workspace"])?;
            check_placeholders(&root)
        }
        Some(other) => bail!("unknown xtask command {other}"),
        None => {
            eprintln!("usage: cargo run -p xtask -- <codegen|check-placeholders|ci>");
            Ok(())
        }
    }
}
