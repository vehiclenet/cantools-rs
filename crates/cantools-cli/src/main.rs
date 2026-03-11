//! `cantools` CLI entrypoint.

use std::{fs, io, path::PathBuf, thread, time::Duration};

use anyhow::{Context, Result, bail};
use can_dbc::Dbc;
use cantools_codec::read_path;
use cantools_core::{CanFrame, CanId, FrameClass, FrameSink, FrameSource, ReplayMode, Timestamp};
use cantools_dbc::Decoder;
use cantools_socketcan::RawSocket;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Generator, Shell, generate};
use clap_mangen::Man;
use serde::Deserialize;

#[derive(Parser)]
#[command(name = "cantools", version, about = "Rust CAN toolkit")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Send(SendArgs),
    Dump(DumpArgs),
    Play(PlayArgs),
    Sequence(SequenceArgs),
    Gen(GenArgs),
    Monitor(MonitorArgs),
}

#[derive(clap::Args)]
struct SendArgs {
    #[arg(long)]
    interface: String,
    #[arg(long)]
    id: String,
    #[arg(long, default_value = "")]
    data: String,
    #[arg(long)]
    fd: bool,
    #[arg(long)]
    brs: bool,
    #[arg(long)]
    esi: bool,
}

#[derive(clap::Args)]
struct DumpArgs {
    #[arg(long)]
    interface: String,
    #[arg(long, default_value_t = 20)]
    count: usize,
    #[arg(long = "dbc")]
    dbc_paths: Vec<PathBuf>,
}

#[derive(clap::Args)]
struct PlayArgs {
    #[arg(long)]
    interface: String,
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    immediate: bool,
    #[arg(long)]
    scale: Option<f64>,
}

#[derive(clap::Args)]
struct SequenceArgs {
    #[arg(long)]
    interface: String,
    #[arg(long)]
    file: PathBuf,
}

#[derive(clap::Args)]
struct MonitorArgs {
    #[arg(long)]
    interface: String,
    #[arg(long, default_value_t = 25)]
    limit: usize,
}

#[derive(clap::Args)]
struct GenArgs {
    #[command(subcommand)]
    command: GenCommand,
}

#[derive(Subcommand)]
enum GenCommand {
    Report {
        #[arg(long)]
        dbc: PathBuf,
    },
    Completions {
        #[arg(value_enum)]
        shell: ShellValue,
    },
    Manpage,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ShellValue {
    Bash,
    Elvish,
    Fish,
    PowerShell,
    Zsh,
}

impl From<ShellValue> for Shell {
    fn from(value: ShellValue) -> Self {
        match value {
            ShellValue::Bash => Shell::Bash,
            ShellValue::Elvish => Shell::Elvish,
            ShellValue::Fish => Shell::Fish,
            ShellValue::PowerShell => Shell::PowerShell,
            ShellValue::Zsh => Shell::Zsh,
        }
    }
}

#[derive(Debug, Deserialize)]
struct SequenceFile {
    steps: Vec<SequenceStep>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum SequenceStep {
    Send {
        id: String,
        data: String,
        fd: Option<bool>,
        brs: Option<bool>,
        esi: Option<bool>,
    },
    Sleep {
        millis: u64,
    },
    Expect {
        id: String,
        timeout_ms: u64,
    },
}

fn parse_hex_bytes(input: &str) -> Result<Vec<u8>> {
    let compact = input.replace([' ', '_'], "");
    if compact.is_empty() {
        return Ok(Vec::new());
    }
    if !compact.len().is_multiple_of(2) {
        bail!("hex payload must contain an even number of digits");
    }

    (0..compact.len())
        .step_by(2)
        .map(|offset| {
            u8::from_str_radix(&compact[offset..offset + 2], 16).context("invalid hex payload byte")
        })
        .collect()
}

fn parse_id(input: &str) -> Result<CanId> {
    let trimmed = input.trim_start_matches("0x");
    let raw = u32::from_str_radix(trimmed, 16)
        .with_context(|| format!("invalid CAN identifier {input}"))?;
    if raw <= 0x7ff {
        Ok(CanId::standard(raw as u16)?)
    } else {
        Ok(CanId::extended(raw)?)
    }
}

fn build_frame(id: &str, data: &str, fd: bool, brs: bool, esi: bool) -> Result<CanFrame> {
    Ok(CanFrame::new(
        parse_id(id)?,
        FrameClass::Data,
        parse_hex_bytes(data)?,
        fd,
        cantools_core::FdFlags {
            bit_rate_switch: brs,
            error_state_indicator: esi,
        },
    )?)
}

fn render_frame(frame: &CanFrame) -> String {
    let id = if frame.id.is_extended() {
        format!("{:08X}", frame.id.raw())
    } else {
        format!("{:03X}", frame.id.raw())
    };
    let data = frame
        .data
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let suffix = if frame.fd { " fd" } else { "" };
    format!("{id} [{:>2}] {data}{suffix}", frame.data.len())
}

fn replay_mode(immediate: bool, scale: Option<f64>) -> ReplayMode {
    if immediate {
        ReplayMode::Immediate
    } else if let Some(scale) = scale {
        ReplayMode::Scale(scale)
    } else {
        ReplayMode::Preserve
    }
}

fn load_decoder(paths: &[PathBuf]) -> Result<Option<Decoder>> {
    if paths.is_empty() {
        return Ok(None);
    }
    let mut decoder = Decoder::new();
    for path in paths {
        decoder.push_file(path)?;
    }
    Ok(Some(decoder))
}

fn replay(
    socket: &mut RawSocket,
    events: &[cantools_core::CaptureEvent],
    mode: ReplayMode,
) -> Result<()> {
    let mut previous: Option<Timestamp> = None;
    for event in events {
        if let Some(previous_timestamp) = previous {
            let current =
                event.timestamp.seconds as i128 * 1_000_000_000 + i128::from(event.timestamp.nanos);
            let previous = previous_timestamp.seconds as i128 * 1_000_000_000
                + i128::from(previous_timestamp.nanos);
            let delta_ns = (current - previous).max(0) as u64;
            match mode {
                ReplayMode::Immediate => {}
                ReplayMode::Preserve => thread::sleep(Duration::from_nanos(delta_ns)),
                ReplayMode::Scale(scale) => thread::sleep(Duration::from_nanos(
                    (delta_ns as f64 / scale.max(0.000_1)) as u64,
                )),
            }
        }
        socket.send(&event.frame)?;
        previous = Some(event.timestamp);
    }
    Ok(())
}

fn generate_completions<G: Generator>(generator: G) {
    let mut command = Cli::command();
    generate(generator, &mut command, "cantools", &mut io::stdout());
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Send(args) => {
            let frame = build_frame(&args.id, &args.data, args.fd, args.brs, args.esi)?;
            let mut socket = RawSocket::open(&args.interface)?;
            socket.send(&frame)?;
            println!("{}", render_frame(&frame));
        }
        Command::Dump(args) => {
            let decoder = load_decoder(&args.dbc_paths)?;
            let mut socket = RawSocket::open(&args.interface)?;
            for _ in 0..args.count {
                if let Some(frame) = socket.recv()? {
                    let rendered = render_frame(&frame);
                    if let Some(decoder) = &decoder {
                        let outcome = decoder.decode_frame(&frame);
                        if let Some(message) = outcome.message {
                            let signals = message
                                .signals
                                .iter()
                                .map(|signal| format!("{}={}", signal.name, signal.scaled))
                                .collect::<Vec<_>>()
                                .join(", ");
                            println!("{rendered}  {}", signals);
                        } else {
                            println!("{rendered}");
                        }
                    } else {
                        println!("{rendered}");
                    }
                }
            }
        }
        Command::Play(args) => {
            let mut socket = RawSocket::open(&args.interface)?;
            let report = read_path(&args.input)?;
            replay(
                &mut socket,
                &report.events,
                replay_mode(args.immediate, args.scale),
            )?;
            for note in report.notes {
                eprintln!("note: {}", note.detail);
            }
        }
        Command::Sequence(args) => {
            let content = fs::read_to_string(&args.file)?;
            let sequence: SequenceFile = toml::from_str(&content)?;
            let mut socket = RawSocket::open(&args.interface)?;
            socket.set_nonblocking(true)?;
            for step in sequence.steps {
                match step {
                    SequenceStep::Send {
                        id,
                        data,
                        fd,
                        brs,
                        esi,
                    } => {
                        let frame = build_frame(
                            &id,
                            &data,
                            fd.unwrap_or(false),
                            brs.unwrap_or(false),
                            esi.unwrap_or(false),
                        )?;
                        socket.send(&frame)?;
                    }
                    SequenceStep::Sleep { millis } => thread::sleep(Duration::from_millis(millis)),
                    SequenceStep::Expect { id, timeout_ms } => {
                        let expected = parse_id(&id)?;
                        let deadline =
                            std::time::Instant::now() + Duration::from_millis(timeout_ms);
                        loop {
                            if std::time::Instant::now() > deadline {
                                bail!("timed out waiting for frame {id}");
                            }
                            if let Some(frame) = socket.recv()? {
                                if frame.id == expected {
                                    break;
                                }
                            } else {
                                thread::sleep(Duration::from_millis(10));
                            }
                        }
                    }
                }
            }
        }
        Command::Gen(args) => match args.command {
            GenCommand::Report { dbc } => {
                let dbc_text = fs::read_to_string(&dbc)?;
                let dbc = Dbc::try_from(dbc_text.as_str())?;
                for message in &dbc.messages {
                    println!(
                        "{} {:>8} bytes {}",
                        message.name,
                        message.size,
                        message.id.raw()
                    );
                    for signal in &message.signals {
                        println!(
                            "  {} start={} size={} unit={}",
                            signal.name, signal.start_bit, signal.size, signal.unit
                        );
                    }
                }
            }
            GenCommand::Completions { shell } => generate_completions(Shell::from(shell)),
            GenCommand::Manpage => {
                let command = Cli::command();
                Man::new(command).render(&mut io::stdout())?;
            }
        },
        Command::Monitor(args) => {
            #[cfg(feature = "monitor")]
            {
                cantools_tui::run_monitor(&args.interface, args.limit)?;
            }
            #[cfg(not(feature = "monitor"))]
            {
                let _ = args;
                bail!("monitor support was disabled at build time");
            }
        }
    }

    Ok(())
}
