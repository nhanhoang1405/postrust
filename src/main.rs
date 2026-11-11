mod app;
mod clipboard;
mod http;
mod launcher;
mod model;
mod output;
mod ui;

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyEventKind,
    },
    execute,
};
use std::{io, path::PathBuf, time::Duration};

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
        ratatui::restore();
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut workspace_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "postrust — terminal HTTP workbench\n\nUsage: postrust [--workspace PATH]\n\nF1 help · F2 workspaces · F3 environment · F5 send · Ctrl+S save · Ctrl+Q quit\nMouse, Tab, arrows, Ctrl+C copy, Ctrl+V paste.\nMinimum terminal: 72 × 26.\nStartup: create or enter a workspace"
                );
                return Ok(());
            }
            "--version" | "-V" => {
                println!("postrust {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--workspace" => {
                workspace_path = Some(PathBuf::from(
                    args.next().context("--workspace requires a path")?,
                ))
            }
            _ => bail!("Unknown option: {arg}. Use --help"),
        }
    }
    let mut launcher = launcher::Launcher::new()?;
    let mut active = if let Some(path) = workspace_path {
        if path.exists() {
            let workspace = model::Workspace::load(&path)?;
            let path = std::fs::canonicalize(path)?;
            launcher.remember(path.clone());
            Some(app::App::new(workspace, path)?)
        } else {
            launcher.prefill(path);
            None
        }
    } else {
        None
    };
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    loop {
        if let Some(app) = &mut active {
            app.poll().await;
        }
        terminal.draw(|frame| {
            if let Some(app) = &mut active {
                app.draw(frame);
            } else {
                launcher.draw(frame);
            }
        })?;
        if event::poll(Duration::from_millis(30))? {
            let event = event::read()?;
            if let Some(app) = &mut active {
                match event {
                    Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
                    Event::Mouse(mouse) => app.mouse(mouse),
                    Event::Paste(text) => app.paste(&text),
                    _ => {}
                }
            } else {
                match event {
                    Event::Key(key) if key.kind != KeyEventKind::Release => launcher.key(key),
                    Event::Mouse(mouse) => launcher.mouse(mouse),
                    Event::Paste(text) => launcher.paste(&text),
                    _ => {}
                }
            }
        }
        if active.as_ref().is_some_and(|app| app.should_quit())
            || active.is_none() && launcher.should_quit()
        {
            break;
        }
        if active.as_ref().is_some_and(|app| app.should_leave()) {
            active = None;
            launcher.return_to_list();
        }
        if active.is_none()
            && let Some((workspace, path)) = launcher.take_opened()
        {
            active = Some(app::App::new(workspace, path)?);
        }
    }
    Ok(())
}
