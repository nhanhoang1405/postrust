use crate::{
    clipboard::TextClipboard,
    model::{self, Workspace},
    ui::*,
};
use anyhow::{Context, Result};
use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers as M, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{prelude::*, widgets::*};
use ratatui_textarea::TextArea;
use std::{fs, path::PathBuf};

pub struct Launcher {
    root: PathBuf,
    paths: Vec<PathBuf>,
    names: Vec<String>,
    mode: usize,
    focus: usize,
    name: TextArea<'static>,
    path: TextArea<'static>,
    selected: usize,
    list_state: ListState,
    clipboard: TextClipboard,
    status: String,
    quit: bool,
    pending: Option<(Workspace, PathBuf)>,
    tabs: Vec<Rect>,
    fields: Vec<(usize, Rect)>,
    action: Rect,
    list: Rect,
}
impl Launcher {
    pub fn new() -> Result<Self> {
        Self::at(model::config_dir()?)
    }
    fn at(root: PathBuf) -> Result<Self> {
        let mut launcher = Self {
            root,
            paths: vec![],
            names: vec![],
            mode: 1,
            focus: 1,
            name: text(""),
            path: text(""),
            selected: 0,
            list_state: ListState::default(),
            clipboard: TextClipboard::default(),
            status: "Create a workspace or open an existing workspace file".into(),
            quit: false,
            pending: None,
            tabs: vec![],
            fields: vec![],
            action: Rect::default(),
            list: Rect::default(),
        };
        launcher.refresh();
        if !launcher.paths.is_empty() {
            launcher.mode = 0;
        }
        Ok(launcher)
    }
    pub fn refresh(&mut self) {
        let mut paths: Vec<PathBuf> = fs::read(self.root.join("recent-workspaces.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if let Ok(entries) = fs::read_dir(self.root.join("workspaces")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json")
                    && !path.to_string_lossy().ends_with("v1-backup.json")
                {
                    paths.push(path);
                }
            }
        }
        let legacy = self.root.join("workspace.json");
        if legacy.is_file() {
            paths.push(legacy);
        }
        self.paths.clear();
        for path in paths {
            if path.is_file() && !self.paths.contains(&path) {
                self.paths.push(path);
            }
        }
        self.names = self.paths.iter().map(|p| workspace_title(p)).collect();
        self.selected = self.selected.min(self.paths.len().saturating_sub(1));
    }
    pub fn return_to_list(&mut self) {
        self.refresh();
        self.mode = 0;
        self.focus = 1;
        self.status = "Choose a workspace, create a new one, or open a file".into();
    }
    pub fn prefill(&mut self, path: PathBuf) {
        self.mode = 1;
        self.path = text(&path.to_string_lossy());
        self.name = text(&path.file_stem().unwrap_or_default().to_string_lossy());
    }
    pub fn should_quit(&self) -> bool {
        self.quit
    }
    pub fn take_opened(&mut self) -> Option<(Workspace, PathBuf)> {
        self.pending.take()
    }
    pub fn remember(&mut self, path: PathBuf) {
        self.paths.retain(|p| p != &path);
        self.paths.insert(0, path);
        self.paths.truncate(30);
        self.names = self.paths.iter().map(|p| workspace_title(p)).collect();
        let result = (|| -> Result<()> {
            fs::create_dir_all(&self.root)?;
            let temporary = self.root.join("recent-workspaces.tmp");
            model::private_write(&temporary, &serde_json::to_vec_pretty(&self.paths)?)?;
            fs::rename(temporary, self.root.join("recent-workspaces.json"))?;
            Ok(())
        })();
        if let Err(error) = result {
            self.status = format!("Workspace opened; could not save recent list: {error:#}");
        }
    }
    fn activate(&mut self) {
        let result = (|| -> Result<(Workspace, PathBuf)> {
            let path = if self.mode == 0 {
                self.paths
                    .get(self.selected)
                    .context("No saved workspace; choose Create or Open file")?
                    .clone()
            } else if value(&self.path).trim().is_empty() && self.mode == 1 {
                let name = value(&self.name);
                let slug: String = name
                    .trim()
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() || c == '-' || c == '_' {
                            c.to_ascii_lowercase()
                        } else {
                            '-'
                        }
                    })
                    .collect();
                self.root.join("workspaces").join(format!("{slug}.json"))
            } else {
                let path = value(&self.path);
                if path.trim().is_empty() {
                    anyhow::bail!("Enter a workspace file path");
                }
                PathBuf::from(path.trim())
            };
            let workspace = if self.mode == 1 {
                let workspace = Workspace {
                    name: value(&self.name).trim().into(),
                    ..Workspace::default()
                };
                if workspace.name.is_empty() {
                    anyhow::bail!("Enter a workspace name");
                }
                workspace.create(&path)?;
                workspace
            } else {
                Workspace::load(&path)?
            };
            Ok((workspace, fs::canonicalize(&path)?))
        })();
        match result {
            Ok((workspace, path)) => {
                self.remember(path.clone());
                self.pending = Some((workspace, path));
            }
            Err(error) => self.status = format!("Cannot enter workspace: {error:#}"),
        }
    }
    fn slots(&self) -> usize {
        if self.mode == 1 { 4 } else { 3 }
    }
    fn editor(&mut self) -> Option<&mut TextArea<'static>> {
        match (self.mode, self.focus) {
            (1, 1) => Some(&mut self.name),
            (1, 2) | (2, 1) => Some(&mut self.path),
            _ => None,
        }
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(editor) = self.editor() {
            editor.insert_str(text.replace(['\r', '\n'], ""));
        }
    }
    pub fn key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(M::CONTROL);
        if ctrl && key.code == KeyCode::Char('q') {
            self.quit = true;
            return;
        }
        if ctrl && key.code == KeyCode::Char('c') {
            if let Some(editor) = self.editor() {
                let content = copied(editor);
                self.clipboard.copy(content);
            }
            return;
        }
        if ctrl && key.code == KeyCode::Char('v') {
            match self.clipboard.paste() {
                Ok(content) => self.paste(&content),
                Err(error) => self.status = error.to_string(),
            }
            return;
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                let back = key.code == KeyCode::BackTab || key.modifiers.contains(M::SHIFT);
                self.focus = (self.focus + if back { self.slots() - 1 } else { 1 }) % self.slots();
            }
            KeyCode::F(2) => {
                self.mode = (self.mode + 1) % 3;
                self.focus = 1;
            }
            KeyCode::Left | KeyCode::Right if self.focus == 0 => {
                self.mode = (self.mode + if key.code == KeyCode::Left { 2 } else { 1 }) % 3;
            }
            KeyCode::Up if self.mode == 0 && self.focus == 1 => {
                self.selected = self.selected.saturating_sub(1)
            }
            KeyCode::Down if self.mode == 0 && self.focus == 1 => {
                self.selected = (self.selected + 1).min(self.paths.len().saturating_sub(1))
            }
            KeyCode::Enter
                if self.focus == self.slots() - 1 || (self.mode == 0 && self.focus == 1) =>
            {
                self.activate()
            }
            KeyCode::Enter => self.focus = (self.focus + 1) % self.slots(),
            _ => {
                if let Some(editor) = self.editor() {
                    edit(editor, key);
                }
            }
        }
    }
    pub fn mouse(&mut self, event: MouseEvent) {
        let point = Position::new(event.column, event.row);
        if event.kind == MouseEventKind::Down(MouseButton::Left) {
            for (i, rect) in self.tabs.iter().enumerate() {
                if rect.contains(point) {
                    self.mode = i;
                    self.focus = 1;
                    return;
                }
            }
            if self.action.contains(point) {
                self.focus = self.slots() - 1;
                self.activate();
                return;
            }
            if self.mode == 0 && self.list.inner(Margin::new(1, 1)).contains(point) {
                let index = self.list_state.offset() + (point.y - self.list.y - 1) as usize;
                if index < self.paths.len() {
                    self.selected = index;
                    self.activate();
                }
                return;
            }
            for &(focus, rect) in &self.fields {
                if rect.contains(point) {
                    self.focus = focus;
                    if let Some(editor) = self.editor() {
                        editor.cancel_selection();
                        click(editor, rect, point);
                    }
                    return;
                }
            }
        }
        if self.mode == 0 && self.list.contains(point) {
            match event.kind {
                MouseEventKind::ScrollDown => {
                    self.selected = (self.selected + 1).min(self.paths.len().saturating_sub(1))
                }
                MouseEventKind::ScrollUp => self.selected = self.selected.saturating_sub(1),
                _ => {}
            }
        }
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        background(frame);
        self.tabs.clear();
        self.fields.clear();
        self.action = Rect::default();
        self.list = Rect::default();
        let area = frame.area();
        if area.width < 72 || area.height < 26 {
            frame.render_widget(
                Paragraph::new("Resize to at least 72 × 26. Ctrl+Q quits."),
                area,
            );
            return;
        }
        let regions = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(17),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new(" postrust / HTTP workbench").style(Style::default().fg(ACCENT).bold()),
            regions[0],
        );
        let outer = centered(regions[1], 88.min(area.width - 4), 19);
        frame.render_widget(panel(" Enter a workspace ", true), outer);
        let sections = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(outer.inner(Margin::new(2, 1)));
        self.tabs = Layout::horizontal([Constraint::Ratio(1, 3); 3])
            .split(sections[0])
            .to_vec();
        for (i, label) in ["Workspaces", "Create new", "Open file"].iter().enumerate() {
            button(frame, label, self.tabs[i], self.mode == i);
        }
        if self.mode == 0 {
            self.list = sections[1];
            self.list_state.select(if self.paths.is_empty() {
                None
            } else {
                Some(self.selected)
            });
            let items: Vec<_> = self
                .names
                .iter()
                .map(|name| ListItem::new(name.clone()))
                .collect();
            frame.render_stateful_widget(
                List::new(items)
                    .block(panel(" Your workspaces ", self.focus == 1))
                    .highlight_style(selected())
                    .highlight_symbol("› "),
                sections[1],
                &mut self.list_state,
            );
            if self.paths.is_empty() {
                frame.render_widget(
                    Paragraph::new("No workspaces yet. Choose Create new."),
                    sections[1].inner(Margin::new(1, 1)),
                );
            }
        } else {
            let rows = Layout::vertical([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(1),
            ])
            .split(sections[1]);
            if self.mode == 1 {
                field(
                    frame,
                    &mut self.name,
                    " Workspace name ",
                    rows[0],
                    self.focus == 1,
                );
                field(
                    frame,
                    &mut self.path,
                    " File path (blank: automatic) ",
                    rows[1],
                    self.focus == 2,
                );
                self.fields = vec![(1, rows[0]), (2, rows[1])];
            } else {
                field(
                    frame,
                    &mut self.path,
                    " Existing workspace file path ",
                    rows[0],
                    self.focus == 1,
                );
                self.fields = vec![(1, rows[0])];
                frame.render_widget(Paragraph::new("Open a workspace JSON file. Each workspace keeps its own collections, requests and environment.").wrap(Wrap{trim:false}).style(Style::default().fg(MUTED)),rows[1]);
            }
        }
        self.action = sections[2];
        button(
            frame,
            if self.mode == 1 {
                "Create workspace  ↵"
            } else {
                "Enter workspace  ↵"
            },
            self.action,
            self.focus == self.slots() - 1,
        );
        let status = if self.mode == 0 && !self.status.starts_with("Cannot") {
            self.paths
                .get(self.selected)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| self.status.clone())
        } else {
            self.status.clone()
        };
        frame.render_widget(
            Paragraph::new(format!(" {status}"))
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(MUTED)),
            regions[2],
        );
        frame.render_widget(
            Paragraph::new(
                " Tab focus · F2 choose method · Enter next / enter workspace · Ctrl+Q quit",
            )
            .style(Style::default().fg(MUTED)),
            regions[3],
        );
    }
}
fn workspace_title(path: &std::path::Path) -> String {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creation_opening_and_missing_files_use_explicit_paths() {
        let root = std::env::temp_dir().join(format!("postrust-launcher-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut launcher = Launcher::at(root.clone()).unwrap();
        launcher.name = text("School API");
        launcher.activate();
        let (workspace, path) = launcher.take_opened().unwrap();
        assert_eq!(workspace.name, "School API");
        assert!(path.is_file());
        launcher.activate();
        assert!(launcher.take_opened().is_none());
        assert!(launcher.status.contains("unused path"));
        launcher.mode = 2;
        launcher.path = text(&path.to_string_lossy());
        launcher.activate();
        assert!(launcher.take_opened().is_some());
        launcher.path = text(&root.join("missing.json").to_string_lossy());
        launcher.activate();
        assert!(launcher.take_opened().is_none());
        assert!(!root.join("missing.json").exists());
        let mut reloaded = Launcher::at(root.clone()).unwrap();
        assert!(reloaded.paths.contains(&path));
        for (width, height) in [(72, 26), (80, 26), (120, 40)] {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| reloaded.draw(frame)).unwrap();
            assert_eq!(reloaded.action.height, 3);
            for rect in &reloaded.tabs {
                assert_eq!(rect.height, 3);
                assert!(rect.width >= 18);
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}
