use crate::{
    clipboard::TextClipboard,
    http::{self, Response},
    model::{Environment, HistoryEntry, METHODS, Request, Workspace},
    output::Output,
    ui::*,
};
use anyhow::Result;
use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers as M, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{prelude::*, widgets::*};
use ratatui_textarea::TextArea;
use std::{collections::BTreeSet, path::PathBuf};
use tokio::task::JoinHandle;

const TABS: [&str; 4] = ["Params", "Headers", "Body", "Auth"];
const FIELDS: [&str; 7] = [
    " Request name ",
    " Collection ",
    " URL ",
    " Params · key=value ",
    " Headers · Name: value ",
    " Body · raw text / JSON ",
    " Bearer token ",
];
const HELP: &str = "WORKSPACES\nF2 / Ctrl+W      Return to the workspace chooser\nF3               Edit this workspace's environment (JSON object)\nCtrl+S           Save requests, collections, environment and history\n\nCOLLECTIONS\nUp / Down        Navigate folders and requests\nLeft / Right     Collapse / expand a folder; Left goes to parent\nEnter / click    Toggle a folder or open a request\nCtrl+G           Create a collection (empty folders are retained)\nF7               Rename the selected collection\nDelete           Delete selected request or collection (asks first)\nCtrl+N / Ctrl+D  New / duplicate request in the selected collection\nCollection field Enter / click to move request to another collection\nF8               Switch Collections / History\n\nREQUESTS\nTab / Shift+Tab  Tree → Method → URL → Editor → Output\nAlt+1/2/3/4      Params / Headers / Body / Auth\nF4 / Shift+F4    Next / previous request editor tab\nF5 / Ctrl+Enter  Send asynchronously\nEsc              Cancel running request, otherwise focus tree\nF6               Switch response Body / Headers\nArrows / PgUp/Dn Scroll response (Ctrl/Alt+arrows: faster)\nHome / End       First / last response line\n\nCLIPBOARD\nCtrl+C / Ctrl+V  Copy / paste in text fields\nCtrl+A           Select all text, including output\nShift+arrows     Select output text; Ctrl+C copies selection\nCtrl+Z / Ctrl+Y  Undo / redo (replacement uses two steps)\nMouse            Click controls and text; drag text to select\nWheel            Scroll panel under pointer; Shift scrolls sideways\n\nENVIRONMENT\nEach workspace owns one variable map. There is no profile switch.\n{{variable}} works in URL, params, headers, body and bearer auth.\nWorkspace values override process environment variables.\nUndefined variables prevent sending. Tokens are masked in Auth.\n\nHTTP\nHeaders: Name: value; params: key=value (one per line).\nBlank lines and # comments are ignored in headers/params.\nNew requests use accept: */* and Content-Type: application/json.\nHTTP errors and redirects remain inspectable. Timeout: 30 seconds.\nResponse preview is capped at 2 MiB; JSON is pretty printed.\nHistory retains 50 attempts, without persisted response bodies.\n\nCtrl+Q quits. Unsaved edits prompt Save / Discard / Cancel.\nWorkspace files can contain credentials in plain text (Unix: 0600).\nF1 / Esc closes help.";
#[derive(Clone, Copy, Debug, PartialEq)]
enum Focus {
    Requests,
    Name,
    Collection,
    Method,
    Url,
    Send,
    Editor,
    Response,
}
const FOCUS: [Focus; 5] = [
    Focus::Requests,
    Focus::Method,
    Focus::Url,
    Focus::Editor,
    Focus::Response,
];
#[derive(Clone, Debug, PartialEq)]
enum Node {
    Collection(String),
    Request(usize),
}
enum DialogKind {
    Environment,
    NewCollection,
    RenameCollection(String),
    Move,
    Delete(Node),
    Leave(bool),
    Help,
}
struct Dialog {
    kind: DialogKind,
    editor: TextArea<'static>,
    focus: usize,
    selected: usize,
    state: ListState,
    scroll: u16,
    error: String,
}
impl Dialog {
    fn new(kind: DialogKind, content: &str) -> Self {
        Self {
            kind,
            editor: text(content),
            focus: 0,
            selected: 0,
            state: ListState::default(),
            scroll: 0,
            error: String::new(),
        }
    }
}
struct Flight {
    task: JoinHandle<Result<Response>>,
    request: Request,
}
#[derive(Clone, Copy)]
enum Action {
    Workspace,
    Environment,
    Save,
    Help,
    NewRequest,
    NewCollection,
    Rename,
    Delete,
    Collections,
    History,
    Focus(Focus),
    RequestTab(usize),
    ResponseTab(usize),
}
pub struct App {
    workspace: Workspace,
    path: PathBuf,
    fields: Vec<TextArea<'static>>,
    current: usize,
    method: usize,
    focus: Focus,
    tab: usize,
    response_tab: usize,
    response: Response,
    response_label: String,
    output: Output,
    flight: Option<Flight>,
    client: reqwest::Client,
    clipboard: TextClipboard,
    status: String,
    dirty: bool,
    quit: bool,
    leave: bool,
    history_mode: bool,
    history_selected: usize,
    tree_selected: usize,
    collapsed: BTreeSet<String>,
    list_state: ListState,
    dialog: Option<Dialog>,
    suspended: Option<Dialog>,
    dialog_rect: Rect,
    dialog_buttons: Vec<Rect>,
    dialog_list: Rect,
    field_rects: Vec<Rect>,
    hits: Vec<(Action, Rect)>,
    list_rect: Rect,
    response_rect: Rect,
    dragging: bool,
}
impl App {
    pub fn new(workspace: Workspace, path: PathBuf) -> Result<Self> {
        let mut app = Self {
            workspace,
            path,
            fields: vec![],
            current: 0,
            method: 0,
            focus: Focus::Requests,
            tab: 1,
            response_tab: 0,
            response: Response::default(),
            response_label: "No request sent".into(),
            output: Output::default(),
            flight: None,
            client: http::client()?,
            clipboard: TextClipboard::default(),
            status: "Choose a collection · Ctrl+N new request · Ctrl+G new collection".into(),
            dirty: false,
            quit: false,
            leave: false,
            history_mode: false,
            history_selected: 0,
            tree_selected: 0,
            collapsed: BTreeSet::new(),
            list_state: ListState::default(),
            dialog: None,
            suspended: None,
            dialog_rect: Rect::default(),
            dialog_buttons: vec![],
            dialog_list: Rect::default(),
            field_rects: vec![Rect::default(); 7],
            hits: vec![],
            list_rect: Rect::default(),
            response_rect: Rect::default(),
            dragging: false,
        };
        app.load_fields();
        app.reveal_current();
        Ok(app)
    }
    pub fn should_quit(&self) -> bool {
        self.quit
    }
    pub fn should_leave(&self) -> bool {
        self.leave
    }
    fn has_request(&self) -> bool {
        self.current < self.workspace.requests.len()
    }
    fn load_fields(&mut self) {
        let request = self
            .workspace
            .requests
            .get(self.current)
            .cloned()
            .unwrap_or_default();
        self.fields = [
            &request.name,
            &request.collection,
            &request.url,
            &request.params,
            &request.headers,
            &request.body,
            &request.bearer,
        ]
        .iter()
        .map(|s| text(s))
        .collect();
        self.fields[6].set_mask_char('•');
        self.method = METHODS
            .iter()
            .position(|m| *m == request.method)
            .unwrap_or(0);
    }
    fn draft(&self) -> Request {
        Request {
            name: value(&self.fields[0]),
            collection: value(&self.fields[1]),
            method: METHODS[self.method].into(),
            url: value(&self.fields[2]),
            params: value(&self.fields[3]),
            headers: value(&self.fields[4]),
            body: value(&self.fields[5]),
            bearer: value(&self.fields[6]),
        }
    }
    fn sync(&mut self) {
        if self.has_request() {
            self.workspace.requests[self.current] = self.draft();
        }
    }
    fn field_index(&self) -> Option<usize> {
        if !self.has_request() {
            return None;
        }
        match self.focus {
            Focus::Name => Some(0),
            Focus::Url => Some(2),
            Focus::Editor => Some(3 + self.tab),
            _ => None,
        }
    }
    fn rows(&self) -> Vec<Node> {
        let mut rows = vec![];
        for name in &self.workspace.collections {
            rows.push(Node::Collection(name.clone()));
            if !self.collapsed.contains(name) {
                for (i, request) in self.workspace.requests.iter().enumerate() {
                    if request.collection == *name {
                        rows.push(Node::Request(i));
                    }
                }
            }
        }
        rows
    }
    fn selected_node(&self) -> Option<Node> {
        self.rows().get(self.tree_selected).cloned()
    }
    fn selected_collection(&self) -> Option<String> {
        match self.selected_node() {
            Some(Node::Collection(name)) => Some(name),
            Some(Node::Request(i)) => Some(self.workspace.requests[i].collection.clone()),
            None => self.workspace.collections.first().cloned(),
        }
    }
    fn reveal_current(&mut self) {
        if let Some(request) = self.workspace.requests.get(self.current) {
            self.collapsed.remove(&request.collection);
            self.tree_selected = self
                .rows()
                .iter()
                .position(|n| *n == Node::Request(self.current))
                .unwrap_or(0);
        } else {
            self.tree_selected = self.tree_selected.min(self.rows().len().saturating_sub(1));
        }
    }
    fn select_request(&mut self, index: usize) {
        self.sync();
        self.current = index;
        self.load_fields();
    }
    fn choose_tree(&mut self, index: usize, activate: bool) {
        let rows = self.rows();
        if let Some(node) = rows.get(index) {
            self.tree_selected = index;
            match node {
                Node::Request(i) => {
                    self.select_request(*i);
                }
                Node::Collection(name) if activate && !self.collapsed.remove(name) => {
                    self.collapsed.insert(name.clone());
                }
                _ => {}
            }
        }
    }
    fn save(&mut self) {
        self.sync();
        match self.workspace.save(&self.path) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", self.path.display());
            }
            Err(error) => self.status = format!("Save failed: {error:#}"),
        }
    }
    fn new_request(&mut self, duplicate: bool) {
        self.sync();
        let collection = self
            .selected_collection()
            .unwrap_or_else(|| "My collection".into());
        if !self.workspace.collections.contains(&collection) {
            self.workspace.collections.push(collection.clone());
        }
        let request = if duplicate && self.has_request() {
            let mut request = self.draft();
            request.name.push_str(" (copy)");
            request.collection = collection;
            request
        } else {
            Request {
                collection,
                ..Request::default()
            }
        };
        self.workspace.requests.push(request);
        self.current = self.workspace.requests.len() - 1;
        self.load_fields();
        self.reveal_current();
        self.history_mode = false;
        self.focus = Focus::Name;
        self.fields[0].select_all();
        self.dirty = true;
    }
    fn restore_history(&mut self) {
        if let Some(entry) = self.workspace.history.get(self.history_selected) {
            let request = entry.request.clone();
            self.sync();
            if !self.workspace.collections.contains(&request.collection) {
                self.workspace.collections.push(request.collection.clone());
            }
            self.workspace.requests.push(request);
            self.current = self.workspace.requests.len() - 1;
            self.load_fields();
            self.reveal_current();
            self.history_mode = false;
            self.dirty = true;
            self.status = "Restored history as a new request · Ctrl+S to save".into();
        }
    }
    fn open_dialog(&mut self, kind: DialogKind) {
        let content = match &kind {
            DialogKind::Environment => {
                serde_json::to_string_pretty(&self.workspace.environment).unwrap()
            }
            DialogKind::RenameCollection(name) => name.clone(),
            _ => String::new(),
        };
        self.dialog = Some(Dialog::new(kind, &content));
        if let Some(dialog) = &mut self.dialog {
            if matches!(dialog.kind, DialogKind::RenameCollection(_)) {
                dialog.editor.select_all();
            }
            if matches!(dialog.kind, DialogKind::Move) {
                dialog.selected = self
                    .workspace
                    .collections
                    .iter()
                    .position(|c| *c == value(&self.fields[1]))
                    .unwrap_or(0);
            }
        }
    }
    fn rename(&mut self) {
        if let Some(name) = self.selected_collection() {
            self.open_dialog(DialogKind::RenameCollection(name));
        } else {
            self.status = "Create a collection first (Ctrl+G)".into();
        }
    }
    fn delete(&mut self) {
        if !self.history_mode
            && let Some(node) = self.selected_node()
        {
            self.open_dialog(DialogKind::Delete(node));
        }
    }
    fn move_request(&mut self) {
        if self.has_request() {
            self.open_dialog(DialogKind::Move);
        }
    }
    fn request_leave(&mut self, picker: bool) {
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Leave(_)))
        {
            return;
        }
        if self.dirty || self.dialog.is_some() {
            self.suspended = self.dialog.take();
            self.open_dialog(DialogKind::Leave(picker));
        } else {
            self.finish_leave(picker);
        }
    }
    fn finish_leave(&mut self, picker: bool) {
        if let Some(flight) = self.flight.take() {
            flight.task.abort();
        }
        if picker {
            self.leave = true;
        } else {
            self.quit = true;
        }
    }
    fn apply_dialog(&mut self, dialog: &Dialog) -> bool {
        let result = (|| -> Result<()> {
            match &dialog.kind {
                DialogKind::Environment => {
                    self.workspace.environment =
                        serde_json::from_str::<Environment>(&value(&dialog.editor))?;
                    self.dirty = true;
                    self.save();
                    if self.dirty {
                        anyhow::bail!("Environment applied in memory; save failed");
                    }
                }
                DialogKind::NewCollection => {
                    let name = value(&dialog.editor);
                    self.sync();
                    self.workspace.add_collection(&name)?;
                    self.tree_selected = self
                        .rows()
                        .iter()
                        .position(|n| *n == Node::Collection(name.trim().into()))
                        .unwrap_or(0);
                    self.history_mode = false;
                    self.dirty = true;
                }
                DialogKind::RenameCollection(old) => {
                    self.sync();
                    let current = self.workspace.requests.get(self.current).cloned();
                    let name = value(&dialog.editor);
                    self.workspace.rename_collection(old, &name)?;
                    self.collapsed.remove(old);
                    if current.is_some() {
                        self.load_fields();
                    }
                    self.tree_selected = self
                        .rows()
                        .iter()
                        .position(|n| *n == Node::Collection(name.trim().into()))
                        .unwrap_or(0);
                    self.dirty = true;
                }
                DialogKind::Move => {
                    if let Some(name) = self.workspace.collections.get(dialog.selected).cloned() {
                        self.fields[1] = text(&name);
                        self.sync();
                        self.reveal_current();
                        self.dirty = true;
                    }
                }
                DialogKind::Delete(node) => {
                    self.sync();
                    match node {
                        Node::Collection(name) => self.workspace.delete_collection(name),
                        Node::Request(index) => {
                            self.workspace.requests.remove(*index);
                        }
                    }
                    self.current = self
                        .current
                        .min(self.workspace.requests.len().saturating_sub(1));
                    self.load_fields();
                    self.tree_selected =
                        self.tree_selected.min(self.rows().len().saturating_sub(1));
                    self.dirty = true;
                }
                DialogKind::Leave(picker) => {
                    if let Some(draft) = self.suspended.take()
                        && matches!(
                            draft.kind,
                            DialogKind::Environment
                                | DialogKind::NewCollection
                                | DialogKind::RenameCollection(_)
                        )
                        && !self.apply_dialog(&draft)
                    {
                        self.suspended = Some(draft);
                        anyhow::bail!("Resolve the dialog error before saving");
                    }
                    self.save();
                    if self.dirty {
                        anyhow::bail!("Workspace was not saved");
                    }
                    self.finish_leave(*picker);
                }
                DialogKind::Help => {}
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                if !matches!(dialog.kind, DialogKind::Environment | DialogKind::Leave(_)) {
                    self.status = "Updated · Ctrl+S to save workspace".into();
                }
                true
            }
            Err(error) => {
                self.status = format!("Cannot apply: {error:#}");
                false
            }
        }
    }
    fn dialog_key(&mut self, key: KeyEvent) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let ctrl = key.modifiers.contains(M::CONTROL);
        if key.code == KeyCode::Esc
            || key.code == KeyCode::F(1) && matches!(dialog.kind, DialogKind::Help)
        {
            if matches!(dialog.kind, DialogKind::Leave(_)) {
                self.dialog = self.suspended.take();
            }
            return;
        }
        if matches!(dialog.kind, DialogKind::Help) {
            match key.code {
                KeyCode::Down => dialog.scroll = dialog.scroll.saturating_add(1),
                KeyCode::Up => dialog.scroll = dialog.scroll.saturating_sub(1),
                KeyCode::PageDown => dialog.scroll = dialog.scroll.saturating_add(10),
                KeyCode::PageUp => dialog.scroll = dialog.scroll.saturating_sub(10),
                _ => {}
            }
            dialog.scroll = dialog
                .scroll
                .min(HELP.lines().count().saturating_sub(4) as u16);
            self.dialog = Some(dialog);
            return;
        }
        if matches!(dialog.kind, DialogKind::Leave(_)) {
            if key.code == KeyCode::Char('d') {
                if let DialogKind::Leave(picker) = dialog.kind {
                    self.finish_leave(picker);
                }
                self.suspended = None;
                return;
            }
            if key.code == KeyCode::Char('s') {
                if !self.apply_dialog(&dialog) {
                    let mut restored = self.suspended.take().unwrap_or(dialog);
                    restored.error = self.status.clone();
                    self.dialog = Some(restored);
                }
                return;
            }
        }
        if let DialogKind::Leave(picker) = dialog.kind {
            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    let back = key.code == KeyCode::BackTab || key.modifiers.contains(M::SHIFT);
                    dialog.focus = (dialog.focus + if back { 3 } else { 1 }) % 4;
                }
                KeyCode::Enter if dialog.focus == 2 => {
                    self.finish_leave(picker);
                    self.suspended = None;
                    return;
                }
                KeyCode::Enter if dialog.focus == 3 => {
                    self.dialog = self.suspended.take();
                    return;
                }
                KeyCode::Enter => {
                    if !self.apply_dialog(&dialog) {
                        dialog.error = self.status.clone();
                        self.dialog = Some(dialog);
                    }
                    return;
                }
                _ => {}
            }
            self.dialog = Some(dialog);
            return;
        }
        let is_editor = matches!(
            dialog.kind,
            DialogKind::Environment | DialogKind::NewCollection | DialogKind::RenameCollection(_)
        );
        let is_move = matches!(dialog.kind, DialogKind::Move);
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                let back = key.code == KeyCode::BackTab || key.modifiers.contains(M::SHIFT);
                dialog.focus = (dialog.focus + if back { 2 } else { 1 }) % 3;
            }
            KeyCode::Char('c') if ctrl && is_editor => {
                self.clipboard.copy(copied(&dialog.editor));
            }
            KeyCode::Char('v') if ctrl && is_editor => {
                if let Ok(content) = self.clipboard.paste() {
                    let content = if matches!(dialog.kind, DialogKind::Environment) {
                        content
                    } else {
                        content.replace(['\n', '\r'], "")
                    };
                    dialog.editor.insert_str(content);
                }
            }
            KeyCode::Up if is_move => dialog.selected = dialog.selected.saturating_sub(1),
            KeyCode::Down if is_move => {
                dialog.selected =
                    (dialog.selected + 1).min(self.workspace.collections.len().saturating_sub(1))
            }
            KeyCode::Enter if dialog.focus == 2 => return,
            KeyCode::Enter
                if dialog.focus == 1 || !matches!(dialog.kind, DialogKind::Environment) =>
            {
                if !self.apply_dialog(&dialog) {
                    dialog.error = self.status.clone();
                    self.dialog = Some(dialog);
                }
                return;
            }
            KeyCode::Char('s') if ctrl => {
                if !self.apply_dialog(&dialog) {
                    dialog.error = self.status.clone();
                    self.dialog = Some(dialog);
                }
                return;
            }
            _ if is_editor && dialog.focus == 0 => {
                edit(&mut dialog.editor, key);
            }
            _ => {}
        }
        self.dialog = Some(dialog);
    }
    fn send(&mut self) {
        if !self.has_request() {
            self.status = "Create a request first (Ctrl+N)".into();
            return;
        }
        if self.flight.is_some() {
            self.status = "Sending… · Esc cancels".into();
            return;
        }
        let request = self.draft();
        match http::build(&self.client, &request, &self.workspace.environment) {
            Ok(built) => {
                self.response_label = format!("{} {}", request.method, request.name);
                self.flight = Some(Flight {
                    task: tokio::spawn(http::execute(self.client.clone(), built)),
                    request,
                });
                self.response = Response::default();
                self.output.set("", "");
                self.status = "Sending… · Esc cancels".into();
            }
            Err(error) => self.status = format!("Cannot send: {error:#}"),
        }
    }
    pub async fn poll(&mut self) {
        if !self.flight.as_ref().is_some_and(|f| f.task.is_finished()) {
            return;
        }
        let flight = self.flight.take().unwrap();
        let outcome = match flight.task.await {
            Ok(Ok(response)) => {
                let summary = format!(
                    "{} · {} ms · {} bytes{}",
                    response.status,
                    response.elapsed_ms,
                    response.bytes,
                    if response.truncated {
                        " · preview limited to 2 MiB"
                    } else {
                        ""
                    }
                );
                self.response = response;
                summary
            }
            Ok(Err(error)) => {
                self.response.body = format!("Request failed: {error:#}");
                self.response.body.clone()
            }
            Err(error) => format!("Request task failed: {error}"),
        };
        self.output.set(&self.response.body, &self.response.headers);
        self.status = outcome.clone();
        self.workspace.history.insert(
            0,
            HistoryEntry {
                request: flight.request,
                outcome,
            },
        );
        self.workspace.history.truncate(50);
        self.dirty = true;
    }
    fn copy(&mut self) {
        let content = if let Some(index) = self.field_index() {
            copied(&self.fields[index])
        } else if self.focus == Focus::Requests {
            if self.history_mode {
                self.workspace
                    .history
                    .get(self.history_selected)
                    .map(|h| h.request.url.clone())
                    .unwrap_or_default()
            } else {
                match self.selected_node() {
                    Some(Node::Collection(name)) => name,
                    Some(Node::Request(index)) => self.workspace.requests[index].url.clone(),
                    None => String::new(),
                }
            }
        } else {
            self.output.copy(self.response_tab)
        };
        self.status = if self.clipboard.copy(content) {
            "Copied to clipboard".into()
        } else {
            "Copied to internal clipboard (desktop clipboard unavailable)".into()
        };
    }
    pub fn paste(&mut self, content: &str) {
        if let Some(dialog) = &mut self.dialog {
            if matches!(
                dialog.kind,
                DialogKind::Environment
                    | DialogKind::NewCollection
                    | DialogKind::RenameCollection(_)
            ) {
                let content = if matches!(dialog.kind, DialogKind::Environment) {
                    content.into()
                } else {
                    content.replace(['\r', '\n'], "")
                };
                dialog.editor.insert_str(content);
            }
            return;
        }
        if let Some(index) = self.field_index() {
            let content = if index <= 2 || index == 6 {
                content.replace(['\r', '\n'], "")
            } else {
                content.replace("\r\n", "\n")
            };
            self.fields[index].insert_str(content);
            self.dirty = true;
        }
    }
    pub fn key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(M::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('q' | 'w')) {
            self.request_leave(key.code == KeyCode::Char('w'));
            return;
        }
        if self.dialog.is_some() {
            self.dialog_key(key);
            return;
        }
        if key.modifiers.contains(M::ALT)
            && let KeyCode::Char(digit @ '1'..='4') = key.code
        {
            self.tab = (digit as u8 - b'1') as usize;
            self.focus = Focus::Editor;
            return;
        }
        match key.code {
            KeyCode::F(1) => self.open_dialog(DialogKind::Help),
            KeyCode::F(2) => self.request_leave(true),
            KeyCode::F(3) => self.open_dialog(DialogKind::Environment),
            KeyCode::F(4) => {
                self.tab = (self.tab
                    + if key.modifiers.contains(M::SHIFT) {
                        3
                    } else {
                        1
                    })
                    % 4;
                self.focus = Focus::Editor;
            }
            KeyCode::F(5) => self.send(),
            KeyCode::Enter if ctrl => self.send(),
            KeyCode::F(6) => {
                self.response_tab = 1 - self.response_tab;
                self.focus = Focus::Response;
            }
            KeyCode::F(7) => self.rename(),
            KeyCode::F(8) => {
                self.history_mode = !self.history_mode;
                self.focus = Focus::Requests;
                *self.list_state.offset_mut() = 0;
            }
            KeyCode::Char('s') if ctrl => self.save(),
            KeyCode::Char('n') if ctrl => self.new_request(false),
            KeyCode::Char('d') if ctrl => self.new_request(true),
            KeyCode::Char('g') if ctrl => self.open_dialog(DialogKind::NewCollection),
            KeyCode::Char('c') if ctrl => self.copy(),
            KeyCode::Char('v') if ctrl => match self.clipboard.paste() {
                Ok(content) => self.paste(&content),
                Err(error) => self.status = error.to_string(),
            },
            KeyCode::Delete if self.focus == Focus::Requests => self.delete(),
            KeyCode::Tab | KeyCode::BackTab => {
                let back = key.code == KeyCode::BackTab || key.modifiers.contains(M::SHIFT);
                self.focus = if let Some(i) = FOCUS.iter().position(|f| *f == self.focus) {
                    FOCUS[(i + if back { FOCUS.len() - 1 } else { 1 }) % FOCUS.len()]
                } else {
                    match self.focus {
                        Focus::Name | Focus::Collection => {
                            if back {
                                Focus::Requests
                            } else {
                                Focus::Method
                            }
                        }
                        _ => {
                            if back {
                                Focus::Url
                            } else {
                                Focus::Editor
                            }
                        }
                    }
                };
            }
            KeyCode::Esc => {
                if let Some(flight) = self.flight.take() {
                    flight.task.abort();
                    self.status = "Request cancelled".into();
                } else {
                    self.focus = Focus::Requests;
                }
            }
            _ => self.focus_key(key),
        }
    }
    fn focus_key(&mut self, key: KeyEvent) {
        if let Some(index) = self.field_index() {
            if (index <= 2 || index == 6) && key.code == KeyCode::Enter {
                self.focus = if index == 0 {
                    Focus::Collection
                } else {
                    Focus::Send
                };
                return;
            }
            if edit(&mut self.fields[index], key) {
                self.dirty = true;
            }
            return;
        }
        let previous = matches!(key.code, KeyCode::Left | KeyCode::Up);
        let next = matches!(key.code, KeyCode::Right | KeyCode::Down);
        let activate = matches!(key.code, KeyCode::Enter | KeyCode::Char(' '));
        match self.focus {
            Focus::Requests => {
                if self.history_mode {
                    if previous {
                        self.history_selected = self.history_selected.saturating_sub(1);
                    }
                    if next {
                        self.history_selected = (self.history_selected + 1)
                            .min(self.workspace.history.len().saturating_sub(1));
                    }
                    if activate {
                        self.restore_history();
                    }
                } else {
                    match key.code {
                        KeyCode::Up => {
                            self.choose_tree(self.tree_selected.saturating_sub(1), false)
                        }
                        KeyCode::Down => self.choose_tree(
                            (self.tree_selected + 1).min(self.rows().len().saturating_sub(1)),
                            false,
                        ),
                        KeyCode::Left => match self.selected_node() {
                            Some(Node::Collection(name)) => {
                                self.collapsed.insert(name);
                            }
                            Some(Node::Request(index)) => {
                                let name = &self.workspace.requests[index].collection;
                                self.tree_selected = self
                                    .rows()
                                    .iter()
                                    .position(|n| *n == Node::Collection(name.clone()))
                                    .unwrap_or(0);
                            }
                            None => {}
                        },
                        KeyCode::Right => {
                            if let Some(Node::Collection(name)) = self.selected_node() {
                                self.collapsed.remove(&name);
                            }
                        }
                        _ if activate => {
                            if matches!(self.selected_node(), Some(Node::Request(_))) {
                                self.focus = Focus::Url;
                            } else {
                                self.choose_tree(self.tree_selected, true);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Focus::Collection if activate => self.move_request(),
            Focus::Method if previous || next || activate => {
                self.method =
                    (self.method + if previous { METHODS.len() - 1 } else { 1 }) % METHODS.len();
                self.dirty = true;
            }
            Focus::Send if activate => self.send(),
            Focus::Response => self.output.key(self.response_tab, key),
            _ => {}
        }
    }
    fn scroll_response(&mut self, rows: i32, cols: i32) {
        self.output.scroll(self.response_tab, rows, cols);
    }
    fn action(&mut self, action: Action) {
        match action {
            Action::Workspace => self.request_leave(true),
            Action::Environment => self.open_dialog(DialogKind::Environment),
            Action::Save => self.save(),
            Action::Help => self.open_dialog(DialogKind::Help),
            Action::NewRequest => self.new_request(false),
            Action::NewCollection => self.open_dialog(DialogKind::NewCollection),
            Action::Rename => self.rename(),
            Action::Delete => self.delete(),
            Action::Collections | Action::History => {
                self.history_mode = matches!(action, Action::History);
                self.focus = Focus::Requests;
                *self.list_state.offset_mut() = 0;
            }
            Action::Focus(focus) => {
                self.focus = focus;
                match focus {
                    Focus::Send => self.send(),
                    Focus::Method => {
                        self.method = (self.method + 1) % METHODS.len();
                        self.dirty = true;
                    }
                    Focus::Collection => self.move_request(),
                    _ => {}
                }
            }
            Action::RequestTab(index) => {
                self.tab = index;
                self.focus = Focus::Editor;
            }
            Action::ResponseTab(index) => {
                self.response_tab = index;
                self.focus = Focus::Response;
            }
        }
    }
    pub fn mouse(&mut self, event: MouseEvent) {
        let point = Position::new(event.column, event.row);
        let down = event.kind == MouseEventKind::Down(MouseButton::Left);
        let wheel = match event.kind {
            MouseEventKind::ScrollDown => Some((3, 0)),
            MouseEventKind::ScrollUp => Some((-3, 0)),
            MouseEventKind::ScrollRight => Some((0, 3)),
            MouseEventKind::ScrollLeft => Some((0, -3)),
            _ => None,
        };
        if self.dialog.is_some() {
            if down && let Some(index) = self.dialog_buttons.iter().position(|r| r.contains(point))
            {
                let kind = &self.dialog.as_ref().unwrap().kind;
                let key = match kind {
                    DialogKind::Leave(_) => match index {
                        0 => KeyCode::Char('s'),
                        1 => KeyCode::Char('d'),
                        _ => KeyCode::Esc,
                    },
                    DialogKind::Help => KeyCode::Esc,
                    _ => {
                        if index == 0 {
                            KeyCode::Char('s')
                        } else {
                            KeyCode::Esc
                        }
                    }
                };
                self.dialog_key(KeyEvent::new(
                    key,
                    if key == KeyCode::Char('s') {
                        M::CONTROL
                    } else {
                        M::NONE
                    },
                ));
                return;
            }
            if let Some(dialog) = &mut self.dialog {
                if matches!(dialog.kind, DialogKind::Move) {
                    if down && self.dialog_list.inner(Margin::new(1, 1)).contains(point) {
                        let index =
                            dialog.state.offset() + (point.y - self.dialog_list.y - 1) as usize;
                        if index < self.workspace.collections.len() {
                            dialog.selected = index;
                        }
                    }
                    if let Some((rows, _)) = wheel {
                        dialog.selected = (dialog.selected as i32 + rows)
                            .clamp(0, self.workspace.collections.len().saturating_sub(1) as i32)
                            as usize;
                    }
                } else if matches!(
                    dialog.kind,
                    DialogKind::Environment
                        | DialogKind::NewCollection
                        | DialogKind::RenameCollection(_)
                ) {
                    if down && self.dialog_rect.contains(point) {
                        dialog.focus = 0;
                        dialog.editor.cancel_selection();
                        click(&mut dialog.editor, self.dialog_rect, point);
                        dialog.editor.start_selection();
                        self.dragging = true;
                    }
                    if event.kind == MouseEventKind::Drag(MouseButton::Left) && self.dragging {
                        click(&mut dialog.editor, self.dialog_rect, point);
                    }
                    if let Some((rows, cols)) = wheel {
                        dialog.editor.scroll((rows as i16, cols as i16));
                    }
                } else if matches!(dialog.kind, DialogKind::Help)
                    && let Some((rows, _)) = wheel
                {
                    dialog.scroll = (i32::from(dialog.scroll) + rows)
                        .clamp(0, HELP.lines().count().saturating_sub(4) as i32)
                        as u16;
                }
            }
            return;
        }
        if down {
            self.dragging = false;
            if self.list_rect.inner(Margin::new(1, 1)).contains(point) {
                self.focus = Focus::Requests;
                let index = self.list_state.offset() + (point.y - self.list_rect.y - 1) as usize;
                if self.history_mode {
                    if index < self.workspace.history.len() {
                        self.history_selected = index;
                        self.restore_history();
                    }
                } else {
                    self.choose_tree(index, true);
                }
                return;
            }
            let action = self
                .hits
                .iter()
                .find(|(_, r)| r.contains(point))
                .map(|(a, _)| *a);
            let Some(action) = action else {
                return;
            };
            self.action(action);
            if let Some(index) = self.field_index()
                && self.field_rects[index].contains(point)
            {
                self.fields[index].cancel_selection();
                click(&mut self.fields[index], self.field_rects[index], point);
                self.fields[index].start_selection();
                self.dragging = true;
            } else if self.focus == Focus::Response {
                self.dragging =
                    self.output
                        .click(self.response_tab, self.response_rect, point, true);
            }
        }
        if event.kind == MouseEventKind::Drag(MouseButton::Left) && self.dragging {
            if let Some(index) = self.field_index() {
                click(&mut self.fields[index], self.field_rects[index], point);
            } else if self.focus == Focus::Response {
                self.output
                    .click(self.response_tab, self.response_rect, point, false);
            }
        }
        if event.kind == MouseEventKind::Up(MouseButton::Left) {
            self.dragging = false;
        }
        if let Some((mut rows, mut cols)) = wheel {
            if event.modifiers.contains(M::SHIFT) {
                cols = rows;
                rows = 0;
            }
            if event.modifiers.intersects(M::CONTROL | M::ALT) {
                rows *= 3;
                cols *= 3;
            }
            if self.response_rect.contains(point) {
                self.scroll_response(rows, cols);
            } else if self.list_rect.contains(point) {
                if self.history_mode {
                    self.history_selected = (self.history_selected as i32 + rows)
                        .clamp(0, self.workspace.history.len().saturating_sub(1) as i32)
                        as usize;
                } else {
                    let index = (self.tree_selected as i32 + rows)
                        .clamp(0, self.rows().len().saturating_sub(1) as i32)
                        as usize;
                    self.choose_tree(index, false);
                }
            } else {
                for (index, rect) in self.field_rects.iter().enumerate() {
                    if rect.contains(point) {
                        self.fields[index].scroll((rows as i16, cols as i16));
                        break;
                    }
                }
            }
        }
    }
    fn draw_button(
        &mut self,
        frame: &mut Frame,
        label: &str,
        rect: Rect,
        action: Action,
        active: bool,
    ) {
        button(frame, label, rect, active);
        self.hits.push((action, rect));
    }
    fn draw_field(&mut self, frame: &mut Frame, index: usize, rect: Rect, focus: Focus) {
        field(
            frame,
            &mut self.fields[index],
            FIELDS[index],
            rect,
            self.focus == focus,
        );
        self.field_rects[index] = rect;
        self.hits.push((Action::Focus(focus), rect));
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        background(frame);
        let area = frame.area();
        self.hits.clear();
        self.field_rects.fill(Rect::default());
        self.list_rect = Rect::default();
        self.response_rect = Rect::default();
        self.dialog_buttons.clear();
        self.dialog_list = Rect::default();
        self.dialog_rect = Rect::default();
        if area.width < 72 || area.height < 26 {
            frame.render_widget(
                Paragraph::new("Resize to at least 72 × 26. Ctrl+Q quits."),
                area,
            );
            if self.dialog.is_some() {
                self.draw_dialog(frame, area);
            }
            return;
        }
        let regions = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(18),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .split(area);
        let top = Layout::horizontal([
            Constraint::Min(10),
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Length(10),
            Constraint::Length(9),
        ])
        .split(regions[0]);
        frame.render_widget(
            Paragraph::new(" postrust / HTTP").style(Style::default().fg(ACCENT).bold()),
            Rect::new(top[0].x, top[0].y + 1, top[0].width, 1),
        );
        for (i, label, action) in [
            (1, "F2 Workspaces", Action::Workspace),
            (2, "F3 Environment", Action::Environment),
            (3, "^S Save", Action::Save),
            (4, "F1 Help", Action::Help),
        ] {
            self.draw_button(frame, label, top[i], action, false);
        }
        frame.render_widget(
            Paragraph::new(format!(
                " {}{}",
                self.workspace.name,
                if self.dirty { " · unsaved" } else { "" }
            ))
            .style(Style::default().fg(MUTED)),
            regions[1],
        );
        let columns = Layout::horizontal([
            Constraint::Length((area.width / 4).clamp(28, 36)),
            Constraint::Min(44),
        ])
        .split(regions[2]);
        self.draw_tree(frame, columns[0]);
        if self.has_request() {
            self.draw_request(frame, columns[1]);
        } else {
            frame.render_widget(Paragraph::new("No request selected\n\nChoose a collection, then click Request or press Ctrl+N.\nUse Folder / Ctrl+G to create a collection.").wrap(Wrap{trim:false}).block(panel(" Request ",false)),columns[1]);
        }
        frame.render_widget(
            Paragraph::new(format!(" {}", self.status))
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(MUTED)),
            regions[3],
        );
        frame.render_widget(
            Paragraph::new(" Tab focus · Alt+1–4 tabs · F5 send · ^C copy · ^V paste · ^Q quit")
                .style(Style::default().fg(MUTED)),
            regions[4],
        );
        if self.dialog.is_some() {
            self.draw_dialog(frame, area);
        }
    }
    fn draw_tree(&mut self, frame: &mut Frame, area: Rect) {
        let parts = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(area);
        let tabs = Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(parts[0]);
        self.draw_button(
            frame,
            "Collections",
            tabs[0],
            Action::Collections,
            !self.history_mode,
        );
        self.draw_button(
            frame,
            "History",
            tabs[1],
            Action::History,
            self.history_mode,
        );
        let create = Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(parts[1]);
        self.draw_button(frame, "+ Request", create[0], Action::NewRequest, false);
        self.draw_button(frame, "+ Folder", create[1], Action::NewCollection, false);
        let manage = Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(parts[2]);
        self.draw_button(frame, "Rename F7", manage[0], Action::Rename, false);
        self.draw_button(frame, "Delete", manage[1], Action::Delete, false);
        self.list_rect = parts[3];
        self.hits.push((Action::Focus(Focus::Requests), parts[3]));
        let items: Vec<_> = if self.history_mode {
            self.workspace
                .history
                .iter()
                .map(|h| ListItem::new(format!("{} {}", h.request.method, h.request.name)))
                .collect()
        } else {
            self.rows()
                .iter()
                .map(|node| match node {
                    Node::Collection(name) => {
                        let count = self
                            .workspace
                            .requests
                            .iter()
                            .filter(|r| r.collection == *name)
                            .count();
                        ListItem::new(format!(
                            "{} {name} ({count})",
                            if self.collapsed.contains(name) {
                                "▸"
                            } else {
                                "▾"
                            }
                        ))
                        .style(Style::default().fg(ACCENT))
                    }
                    Node::Request(index) => {
                        let request = if *index == self.current {
                            self.draft()
                        } else {
                            self.workspace.requests[*index].clone()
                        };
                        let branch = if self
                            .workspace
                            .requests
                            .iter()
                            .skip(index + 1)
                            .any(|r| r.collection == request.collection)
                        {
                            "├"
                        } else {
                            "└"
                        };
                        ListItem::new(format!("  {branch} {} {}", request.method, request.name))
                    }
                })
                .collect()
        };
        self.list_state.select(if items.is_empty() {
            None
        } else {
            Some(if self.history_mode {
                self.history_selected.min(items.len() - 1)
            } else {
                self.tree_selected.min(items.len() - 1)
            })
        });
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(
                    if self.history_mode {
                        " History · Enter restores "
                    } else {
                        " Request tree "
                    },
                    self.focus == Focus::Requests,
                ))
                .highlight_style(selected())
                .highlight_symbol("› "),
            parts[3],
            &mut self.list_state,
        );
        frame.render_widget(
            Paragraph::new(if self.history_mode {
                " Enter / click restores"
            } else {
                " ↑↓ select · ←→ expand\n ^G folder · ^N request"
            })
            .style(Style::default().fg(MUTED)),
            parts[4],
        );
    }
    fn draw_request(&mut self, frame: &mut Frame, area: Rect) {
        let remaining = area.height.saturating_sub(8);
        let editor_h = (remaining * 45 / 100).max(4);
        let response_h = remaining.saturating_sub(editor_h);
        let parts = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(editor_h),
            Constraint::Length(1),
            Constraint::Length(response_h),
        ])
        .split(area);
        let metadata = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(parts[0]);
        self.draw_field(frame, 0, metadata[0], Focus::Name);
        frame.render_widget(
            Paragraph::new(format!("{} ▸", value(&self.fields[1]))).block(panel(
                " Collection · move ",
                self.focus == Focus::Collection,
            )),
            metadata[1],
        );
        self.hits
            .push((Action::Focus(Focus::Collection), metadata[1]));
        let url = Layout::horizontal([
            Constraint::Length(10),
            Constraint::Min(24),
            Constraint::Length(10),
        ])
        .split(parts[1]);
        frame.render_widget(
            Paragraph::new(METHODS[self.method])
                .alignment(Alignment::Center)
                .block(panel(" Method ", self.focus == Focus::Method)),
            url[0],
        );
        self.hits.push((Action::Focus(Focus::Method), url[0]));
        self.draw_field(frame, 2, url[1], Focus::Url);
        self.draw_button(
            frame,
            if self.flight.is_some() {
                "Sending"
            } else {
                "Send F5"
            },
            url[2],
            Action::Focus(Focus::Send),
            self.focus == Focus::Send,
        );
        self.draw_tabs(frame, parts[2], true);
        self.draw_field(frame, self.tab + 3, parts[3], Focus::Editor);
        self.draw_tabs(frame, parts[4], false);
        self.response_rect = parts[5];
        self.hits.push((Action::Focus(Focus::Response), parts[5]));
        let title = if !self.response.status.is_empty() {
            format!(
                " {} · {} ms · {} B{} ",
                self.response.status,
                self.response.elapsed_ms,
                self.response.bytes,
                if self.response.truncated {
                    " · LIMITED"
                } else {
                    ""
                }
            )
        } else {
            format!(" {} ", self.response_label)
        };
        self.output.draw(
            frame,
            self.response_tab,
            parts[5],
            &title,
            self.focus == Focus::Response,
        );
    }
    fn draw_tabs(&mut self, frame: &mut Frame, area: Rect, request: bool) {
        let labels: &[&str] = if request {
            &TABS
        } else {
            &["Response body", "Response headers"]
        };
        let widths: Vec<_> = labels
            .iter()
            .map(|_| Constraint::Ratio(1, labels.len() as u32))
            .collect();
        let rects = Layout::horizontal(widths).split(area);
        for (i, label) in labels.iter().enumerate() {
            let active = i == if request { self.tab } else { self.response_tab };
            frame.render_widget(
                Paragraph::new(if request {
                    format!("{} {label}", i + 1)
                } else {
                    label.to_string()
                })
                .alignment(Alignment::Center)
                .style(if active {
                    selected()
                } else {
                    Style::default().fg(MUTED)
                }),
                rects[i],
            );
            self.hits.push((
                if request {
                    Action::RequestTab(i)
                } else {
                    Action::ResponseTab(i)
                },
                rects[i],
            ));
        }
    }
    fn draw_dialog(&mut self, frame: &mut Frame, area: Rect) {
        let dialog = self.dialog.as_mut().unwrap();
        let height = match dialog.kind {
            DialogKind::Environment | DialogKind::Help => area.height.saturating_sub(4),
            DialogKind::Move => 18,
            DialogKind::Leave(_) | DialogKind::Delete(_) => 11,
            _ => 8,
        };
        let rect = centered(area, area.width.saturating_sub(8).min(86), height);
        frame.render_widget(Clear, rect);
        let parts = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(2),
            Constraint::Length(3),
        ])
        .split(rect);
        self.dialog_rect = parts[0];
        match &dialog.kind {
            DialogKind::Environment => field(
                frame,
                &mut dialog.editor,
                " Workspace environment · JSON object ",
                parts[0],
                dialog.focus == 0,
            ),
            DialogKind::NewCollection => field(
                frame,
                &mut dialog.editor,
                " New collection name ",
                parts[0],
                dialog.focus == 0,
            ),
            DialogKind::RenameCollection(_) => field(
                frame,
                &mut dialog.editor,
                " Rename collection ",
                parts[0],
                dialog.focus == 0,
            ),
            DialogKind::Move => {
                self.dialog_list = parts[0];
                dialog.state.select(Some(dialog.selected));
                let items: Vec<_> = self
                    .workspace
                    .collections
                    .iter()
                    .map(|n| ListItem::new(n.clone()))
                    .collect();
                frame.render_stateful_widget(
                    List::new(items)
                        .block(panel(" Move request to collection ", dialog.focus == 0))
                        .highlight_style(selected())
                        .highlight_symbol("› "),
                    parts[0],
                    &mut dialog.state,
                );
            }
            DialogKind::Delete(node) => {
                let message = match node {
                    Node::Collection(name) => format!(
                        "Delete collection {name}?\nAll {} requests in it will be removed.\nEnter deletes; Esc cancels.",
                        self.workspace
                            .requests
                            .iter()
                            .filter(|r| r.collection == *name)
                            .count()
                    ),
                    Node::Request(index) => format!(
                        "Delete request {}?\nEnter deletes; Esc cancels.",
                        self.workspace.requests[*index].name
                    ),
                };
                frame.render_widget(
                    Paragraph::new(message)
                        .wrap(Wrap { trim: false })
                        .block(panel(" Confirm deletion ", true))
                        .style(Style::default().bg(BG)),
                    parts[0],
                );
            }
            DialogKind::Leave(picker) => {
                frame.render_widget(Paragraph::new(if *picker{"Unsaved changes\nSave before choosing another workspace?\ns: Save · d: Discard · Esc: Cancel"}else{"Unsaved changes\nSave before quitting?\ns: Save · d: Discard · Esc: Cancel"}).wrap(Wrap{trim:false}).block(panel(" Unsaved changes ",true)).style(Style::default().bg(BG)),parts[0]);
            }
            DialogKind::Help => {
                frame.render_widget(
                    Paragraph::new(HELP)
                        .wrap(Wrap { trim: false })
                        .scroll((dialog.scroll, 0))
                        .block(panel(" Help · ↑↓ / wheel scroll ", true))
                        .style(Style::default().bg(BG)),
                    parts[0],
                );
            }
        }
        frame.render_widget(
            Paragraph::new(if dialog.error.is_empty() {
                " Tab focus · Enter apply · Esc cancel"
            } else {
                &dialog.error
            })
            .wrap(Wrap { trim: false })
            .style(
                Style::default()
                    .fg(if dialog.error.is_empty() {
                        MUTED
                    } else {
                        Color::LightRed
                    })
                    .bg(BG),
            ),
            parts[1],
        );
        let labels: &[&str] = match dialog.kind {
            DialogKind::Leave(_) => &["Save", "Discard", "Cancel"],
            DialogKind::Delete(_) => &["Delete", "Cancel"],
            DialogKind::Help => &["Close"],
            DialogKind::Environment => &["Apply & save", "Cancel"],
            _ => &["Apply", "Cancel"],
        };
        let constraints: Vec<_> = labels
            .iter()
            .map(|_| Constraint::Ratio(1, labels.len() as u32))
            .collect();
        self.dialog_buttons = Layout::horizontal(constraints).split(parts[2]).to_vec();
        for (i, label) in labels.iter().enumerate() {
            button(frame, label, self.dialog_buttons[i], dialog.focus == i + 1);
        }
    }
}
impl Drop for App {
    fn drop(&mut self) {
        if let Some(flight) = self.flight.take() {
            flight.task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> App {
        App::new(
            Workspace {
                name: "School API".into(),
                collections: vec!["Accounts".into(), "Orders".into(), "Empty folder".into()],
                requests: vec![
                    Request {
                        name: "List accounts".into(),
                        collection: "Accounts".into(),
                        ..Request::default()
                    },
                    Request {
                        name: "Create account".into(),
                        collection: "Accounts".into(),
                        method: "POST".into(),
                        ..Request::default()
                    },
                    Request {
                        name: "List orders".into(),
                        collection: "Orders".into(),
                        ..Request::default()
                    },
                ],
                ..Workspace::default()
            },
            PathBuf::from("/unused/workspace.json"),
        )
        .unwrap()
    }
    fn key(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, M::NONE));
    }
    fn ctrl(app: &mut App, ch: char) {
        app.key(KeyEvent::new(KeyCode::Char(ch), M::CONTROL));
    }
    fn click_at(app: &mut App, rect: Rect) {
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 1,
            row: rect.y + 1,
            modifiers: M::NONE,
        });
    }
    fn snapshot(terminal: &Terminal<ratatui::backend::TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn rendered_controls_fit_and_remain_aligned_at_supported_terminal_sizes() {
        let mut app = sample();
        std::fs::create_dir_all("target/ui-review").unwrap();
        for (width, height) in [(72, 26), (80, 26), (100, 30), (120, 40), (160, 48)] {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| app.draw(f)).unwrap();
            let screen = snapshot(&terminal);
            for label in [
                "F2 Workspaces",
                "F3 Environment",
                "^S Save",
                "F1 Help",
                "Collections",
                "History",
                "+ Request",
                "+ Folder",
                "Rename F7",
                "Delete",
                "Send F5",
                "Response body",
                "Response headers",
            ] {
                assert!(
                    screen.contains(label),
                    "Missing {label} at {width}x{height}:\n{screen}"
                );
            }
            for (i, (_, rect)) in app.hits.iter().enumerate() {
                assert!(rect.right() <= width && rect.bottom() <= height);
                for (_, other) in app.hits.iter().skip(i + 1) {
                    assert_eq!(
                        rect.intersection(*other).area(),
                        0,
                        "Controls overlap at {width}x{height}: {rect:?} / {other:?}"
                    );
                }
            }
            let send = app
                .hits
                .iter()
                .find(|(action, _)| matches!(action, Action::Focus(Focus::Send)))
                .unwrap()
                .1;
            assert_eq!(send.height, 3);
            assert_eq!(send.y, app.field_rects[2].y);
            assert_eq!(app.field_rects[2].height, 3);
            std::fs::write(
                format!("target/ui-review/workbench-{width}x{height}.txt"),
                screen,
            )
            .unwrap();
            app.open_dialog(DialogKind::Environment);
            terminal.draw(|f| app.draw(f)).unwrap();
            assert!(snapshot(&terminal).contains("Apply & save"));
            assert_eq!(app.dialog_buttons[0].height, 3);
            assert!(app.dialog_buttons[0].y >= app.dialog_rect.bottom());
            std::fs::write(
                format!("target/ui-review/environment-{width}x{height}.txt"),
                snapshot(&terminal),
            )
            .unwrap();
            key(&mut app, KeyCode::Esc);
        }
    }
    #[test]
    fn tree_expands_collapses_and_preserves_drafts_when_navigating() {
        let mut app = sample();
        assert_eq!(app.rows().len(), 6);
        key(&mut app, KeyCode::Up);
        assert_eq!(
            app.selected_node(),
            Some(Node::Collection("Accounts".into()))
        );
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.rows().len(), 4);
        key(&mut app, KeyCode::Right);
        assert_eq!(app.rows().len(), 6);
        key(&mut app, KeyCode::Down);
        assert_eq!(app.current, 0);
        app.focus = Focus::Editor;
        app.tab = 2;
        app.paste("draft body 🌍");
        app.focus = Focus::Requests;
        key(&mut app, KeyCode::Down);
        assert_eq!(app.current, 1);
        key(&mut app, KeyCode::Up);
        assert_eq!(value(&app.fields[5]), "draft body 🌍");
        key(&mut app, KeyCode::Left);
        assert_eq!(
            app.selected_node(),
            Some(Node::Collection("Accounts".into()))
        );
    }
    #[test]
    fn collection_dialogs_create_rename_move_and_confirm_deletion() {
        let mut app = sample();
        ctrl(&mut app, 'g');
        app.paste("Payments");
        key(&mut app, KeyCode::Enter);
        assert!(app.workspace.collections.contains(&"Payments".into()));
        assert_eq!(
            app.selected_node(),
            Some(Node::Collection("Payments".into()))
        );
        ctrl(&mut app, 'n');
        assert_eq!(value(&app.fields[1]), "Payments");
        key(&mut app, KeyCode::F(7));
        app.paste("Billing");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.workspace.requests[3].collection, "Billing");
        assert_eq!(value(&app.fields[1]), "Billing");
        app.focus = Focus::Collection;
        key(&mut app, KeyCode::Enter);
        app.dialog.as_mut().unwrap().selected = 1;
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.workspace.requests[3].collection, "Orders");
        app.tree_selected = app
            .rows()
            .iter()
            .position(|n| *n == Node::Collection("Orders".into()))
            .unwrap();
        app.focus = Focus::Requests;
        key(&mut app, KeyCode::Delete);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.workspace.requests.len(), 4);
        key(&mut app, KeyCode::Delete);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.workspace.requests.len(), 2);
        assert!(!app.workspace.collections.contains(&"Orders".into()));
        assert!(app.workspace.collections.contains(&"Billing".into()));
    }
    #[test]
    fn mouse_collection_and_request_controls_match_the_rendered_tree() {
        let mut app = sample();
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let list = app.list_rect;
        click_at(&mut app, Rect::new(list.x, list.y + 1, list.width, 1));
        assert_eq!(app.current, 0);
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: list.x + 2,
            row: list.y + 1,
            modifiers: M::NONE,
        });
        assert!(app.collapsed.contains("Accounts"));
        terminal.draw(|f| app.draw(f)).unwrap();
        let new = app
            .hits
            .iter()
            .find(|(a, _)| matches!(a, Action::NewCollection))
            .unwrap()
            .1;
        click_at(&mut app, new);
        assert!(matches!(
            app.dialog.as_ref().unwrap().kind,
            DialogKind::NewCollection
        ));
        app.paste("Mouse folder");
        terminal.draw(|f| app.draw(f)).unwrap();
        let apply = app.dialog_buttons[0];
        click_at(&mut app, apply);
        assert!(app.workspace.collections.contains(&"Mouse folder".into()));
    }
    #[test]
    fn workspace_environment_validates_and_dialog_errors_are_visible() {
        let mut app = sample();
        key(&mut app, KeyCode::F(3));
        ctrl(&mut app, 'a');
        app.paste("invalid");
        ctrl(&mut app, 's');
        assert!(app.dialog.is_some());
        assert!(app.workspace.environment.0.contains_key("base_url"));
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(80, 26)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        assert!(snapshot(&terminal).contains("Cannot apply"));
        key(&mut app, KeyCode::Esc);
        ctrl(&mut app, 'g');
        app.paste("Accounts");
        key(&mut app, KeyCode::Enter);
        assert!(
            app.dialog
                .as_ref()
                .unwrap()
                .error
                .contains("already exists")
        );
    }
    #[test]
    fn workspace_switch_guard_preserves_dialog_drafts_when_cancelled() {
        let mut app = sample();
        key(&mut app, KeyCode::F(3));
        ctrl(&mut app, 'a');
        app.paste("{\"token\":\"draft\"}");
        ctrl(&mut app, 'w');
        assert!(matches!(
            app.dialog.as_ref().unwrap().kind,
            DialogKind::Leave(true)
        ));
        ctrl(&mut app, 'w');
        key(&mut app, KeyCode::Esc);
        assert_eq!(
            value(&app.dialog.as_ref().unwrap().editor),
            "{\"token\":\"draft\"}"
        );
        assert!(!app.should_leave());
        ctrl(&mut app, 'w');
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Enter);
        assert!(app.should_leave());
        assert!(!app.should_quit());
    }
    #[test]
    fn saving_when_leaving_does_not_confirm_pending_deletion() {
        let mut app = sample();
        let dir = std::env::temp_dir().join(format!("postrust-leave-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        app.path = dir.join("workspace.json");
        app.tree_selected = 0;
        app.delete();
        ctrl(&mut app, 'w');
        key(&mut app, KeyCode::Char('s'));
        assert!(app.should_leave());
        let saved = Workspace::load(&app.path).unwrap();
        assert_eq!(saved.requests.len(), 3);
        assert!(saved.collections.contains(&"Accounts".into()));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn deleting_every_collection_keeps_an_empty_usable_workspace() {
        let mut app = sample();
        while !app.workspace.collections.is_empty() {
            app.tree_selected = 0;
            app.delete();
            key(&mut app, KeyCode::Enter);
        }
        assert!(app.workspace.requests.is_empty());
        assert!(!app.has_request());
        ctrl(&mut app, 'n');
        assert!(app.has_request());
        assert_eq!(app.workspace.collections.len(), 1);
    }
    #[test]
    fn tab_navigation_has_only_five_stops_and_skips_action_buttons() {
        let mut app = sample();
        for expected in [
            Focus::Method,
            Focus::Url,
            Focus::Editor,
            Focus::Response,
            Focus::Requests,
        ] {
            key(&mut app, KeyCode::Tab);
            assert_eq!(app.focus, expected);
        }
        for expected in [
            Focus::Response,
            Focus::Editor,
            Focus::Url,
            Focus::Method,
            Focus::Requests,
        ] {
            app.key(KeyEvent::new(KeyCode::BackTab, M::SHIFT));
            assert_eq!(app.focus, expected);
        }
        app.focus = Focus::Name;
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Method);
        app.focus = Focus::Collection;
        app.key(KeyEvent::new(KeyCode::BackTab, M::SHIFT));
        assert_eq!(app.focus, Focus::Requests);
        app.focus = Focus::Send;
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Editor);
    }
    #[test]
    fn direct_request_tab_shortcuts_focus_the_editor_without_changing_text() {
        let mut app = sample();
        for origin in FOCUS {
            for (tab, digit) in ['1', '2', '3', '4'].iter().enumerate() {
                app.focus = origin;
                app.key(KeyEvent::new(KeyCode::Char(*digit), M::ALT));
                assert_eq!(app.tab, tab);
                assert_eq!(app.focus, Focus::Editor);
            }
        }
        let before = app.draft();
        key(&mut app, KeyCode::F(4));
        assert_eq!(app.tab, 0);
        app.key(KeyEvent::new(KeyCode::F(4), M::SHIFT));
        assert_eq!(app.tab, 3);
        assert_eq!(app.draft(), before);
    }
    #[test]
    fn output_mouse_selection_copies_only_selected_text_into_the_request() {
        let mut app = sample();
        app.response.body = "response 世界\nsecond line".into();
        app.response.headers = "x-token: value".into();
        app.output.set(&app.response.body, &app.response.headers);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let rect = app.response_rect;
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 1,
            row: rect.y + 1,
            modifiers: M::NONE,
        });
        app.mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: rect.x + 9,
            row: rect.y + 1,
            modifiers: M::NONE,
        });
        app.mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: rect.x + 9,
            row: rect.y + 1,
            modifiers: M::NONE,
        });
        assert_eq!(app.focus, Focus::Response);
        ctrl(&mut app, 'c');
        assert_eq!(app.clipboard.paste().unwrap(), "response");
        app.paste("cannot modify output");
        key(&mut app, KeyCode::Delete);
        key(&mut app, KeyCode::Backspace);
        assert!(!app.dirty);
        assert_eq!(app.output.copy(0), "response");
        app.key(KeyEvent::new(KeyCode::Char('3'), M::ALT));
        ctrl(&mut app, 'v');
        assert_eq!(value(&app.fields[5]), "response");
        key(&mut app, KeyCode::F(6));
        assert_eq!(app.focus, Focus::Response);
        ctrl(&mut app, 'a');
        ctrl(&mut app, 'c');
        assert_eq!(app.clipboard.paste().unwrap(), "x-token: value");
    }
    #[test]
    fn keyboard_clipboard_select_all_and_undo_keep_unicode() {
        let mut app = sample();
        app.focus = Focus::Url;
        app.fields[2] = text("https://example.com/世界");
        ctrl(&mut app, 'a');
        ctrl(&mut app, 'c');
        app.paste("replacement");
        assert_eq!(value(&app.fields[2]), "replacement");
        ctrl(&mut app, 'a');
        ctrl(&mut app, 'v');
        assert_eq!(value(&app.fields[2]), "https://example.com/世界");
        ctrl(&mut app, 'z');
        ctrl(&mut app, 'z');
        assert_eq!(value(&app.fields[2]), "replacement");
        ctrl(&mut app, 'y');
        ctrl(&mut app, 'y');
        assert_eq!(value(&app.fields[2]), "https://example.com/世界");
    }
    #[test]
    fn clicking_footer_does_not_activate_a_previously_focused_button() {
        let mut app = sample();
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        app.focus = Focus::Send;
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 39,
            modifiers: M::NONE,
        });
        assert!(app.flight.is_none());
    }
    #[tokio::test]
    async fn request_cancellation_leaves_ui_responsive() {
        let mut app = sample();
        app.fields[2] = text("http://127.0.0.1:1");
        app.send();
        assert!(app.flight.is_some());
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Esc);
        assert!(app.flight.is_none());
        assert_eq!(app.status, "Request cancelled");
    }
}
