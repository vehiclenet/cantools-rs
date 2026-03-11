//! Operator-facing monitor UI used by `cantools monitor`.

use std::{collections::VecDeque, io::stdout, time::Duration};

use anyhow::Result;
use cantools_core::CaptureEvent;
use cantools_socketcan::RawSocket;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::Rect,
    widgets::{Block, Borders, Paragraph},
};

fn format_event(event: &CaptureEvent) -> String {
    let direction = match event.direction {
        cantools_core::Direction::Rx => "Rx",
        cantools_core::Direction::Tx => "Tx",
    };
    let id = if event.frame.id.is_extended() {
        format!("{:08X}", event.frame.id.raw())
    } else {
        format!("{:03X}", event.frame.id.raw())
    };
    let data = event
        .frame
        .data
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{:>10}.{:09} {direction} {id:<8} {data}",
        event.timestamp.seconds, event.timestamp.nanos
    )
}

/// Run the operator-facing monitor on a SocketCAN interface.
pub fn run_monitor(interface: &str, limit: usize) -> Result<()> {
    let socket = RawSocket::open(interface)?;
    socket.set_nonblocking(true)?;

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_loop(&mut terminal, socket, limit);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    socket: RawSocket,
    limit: usize,
) -> Result<()> {
    let mut events = VecDeque::new();
    loop {
        while let Some(event) = socket.recv_event()? {
            if events.len() == limit {
                events.pop_front();
            }
            events.push_back(event);
        }

        terminal.draw(|frame| render(frame.area(), frame, &events))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && matches!(key.code, KeyCode::Char('q'))
        {
            break;
        }
    }

    Ok(())
}

fn render(area: Rect, frame: &mut ratatui::Frame<'_>, events: &VecDeque<CaptureEvent>) {
    let body = if events.is_empty() {
        "Waiting for CAN traffic...\nPress q to quit.".to_string()
    } else {
        events
            .iter()
            .map(format_event)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let widget = Paragraph::new(body).block(
        Block::default()
            .title("cantools monitor")
            .borders(Borders::ALL),
    );
    frame.render_widget(widget, area);
}
