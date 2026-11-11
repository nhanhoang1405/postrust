# postrust

A Rust terminal HTTP workbench inspired by [ATAC](https://github.com/Julien-cpsn/ATAC), following [dbrust](https://github.com/nhanhoang1405/dbrust)'s application style: a startup form, consistently sized controls, visible focus, simple keyboard navigation, and mouse support.

## Run

Requires Rust 1.88 or newer and a terminal of at least **72 × 26** cells. **120 × 40** is recommended.

```sh
cargo run --release --locked
```

Install from GitHub:

```sh
cargo install --git https://github.com/nhanhoang1405/postrust --locked
postrust
```

Install locally:

```sh
cargo install --path . --locked
postrust
```

The current version is **0.1.0**. See [CHANGELOG.md](CHANGELOG.md) for release notes. GitHub installation is available; the crate is not yet published to crates.io.

## Workspaces

Startup opens the workspace chooser:

1. **Create new:** enter a workspace name and optionally a file path. A blank path stores it in the platform config directory. Enter advances through the fields, then activates Create workspace.
2. **Workspaces:** choose an existing workspace with arrows and Enter, or click it.
3. **Open file:** enter an existing workspace JSON file path.

Tab / Shift+Tab moves focus. Click the method tabs or use F2 to cycle them. Creating a workspace never overwrites an existing file. Opening a missing or invalid file shows an error and stays in the chooser.

**F2 Workspaces** or **Ctrl+W** returns to the chooser from the workbench. Unsaved changes prompt Save / Discard / Cancel. Choosing another workspace changes its requests, collections, history, and environment together.

An explicit path can also be supplied:

```sh
postrust --workspace ./api-workspace.json
```

An existing file opens directly. A missing file opens the Create form with that path filled in, so creation remains explicit.

## Collection tree

The **Collections** tab groups requests under expandable collection folders. Empty collections are retained.

| Control | Action |
| --- | --- |
| Up / Down | Navigate folders and requests |
| Left / Right | Collapse / expand a folder; Left on a request selects its parent |
| Enter / click a folder | Collapse / expand it |
| Enter / click a request | Open it for editing |
| **+ Folder** / Ctrl+G | Create a collection |
| **Rename F7** / F7 | Rename selected collection, including its requests |
| **Delete** / Delete | Delete selected request or collection, with confirmation |
| **+ Request** / Ctrl+N | Create a request in the selected collection |
| Ctrl+D | Duplicate the current request in the selected collection |
| **Collection · move** | Pick another collection for the current request |

Dialogs support Tab, Shift+Tab, Enter, Esc, and clickable Apply / Cancel buttons. Deleting a collection explicitly shows the number of contained requests to be removed. Deleting all requests leaves a usable empty workspace.

The request editor includes a request name, a collection selector, method, URL, and Params / Headers / Body / Auth tabs. Alt+1/2/3/4 selects those tabs directly; F4 / Shift+F4 cycles them. Tab in the workbench visits only Collections, Method, URL, the active request editor, and Output; action buttons, name and collection fields remain clickable. Switching requests preserves drafts in memory. Ctrl+S saves all workspace changes.

## Workspace environment

Each workspace owns **one environment**, which is loaded automatically when entering it. **F3 Environment** opens that workspace's variable map as JSON:

```json
{
  "base_url": "http://localhost:8000",
  "token": "development-token"
}
```

Click **Apply & save** or press Ctrl+S to validate, apply, and save. Esc / Cancel discards the dialog draft. Each new workspace starts with `base_url` set to `http://localhost:8000`.

`{{variable}}` works in URLs, query parameters, headers, bodies, and bearer auth. Workspace values override process environment variables. For example, launch with `API_TOKEN=... postrust`, then use `{{API_TOKEN}}` in Auth. Undefined variables prevent sending and show an error. Substitution is one pass; braces inside a variable's value stay literal.

Older workspace files load their previously active environment as the workspace environment. The first save writes the new format and preserves the original file, including all old profiles, beside it as `*.v1-backup.json`.

## HTTP requests

Click Method or use arrows while it is focused to choose GET, POST, PUT, PATCH, DELETE, HEAD, or OPTIONS. Enter an absolute HTTP(S) URL.

- **Params:** one `key=value` per line. Keys and values are URL encoded; existing URL parameters and repeated keys are retained.
- **Headers:** one `Name: value` per line. New requests start with `accept: */*` and `Content-Type: application/json`. Blank lines and `#` comments are ignored in both Params and Headers.
- **Body:** raw multiline text. JSON Content-Type is supplied by default; change it in Headers for other body formats. URL-encoded form text works with the corresponding Content-Type.
- **Auth:** a masked bearer token, optionally `{{token}}`. Other authentication headers can be entered in Headers; basic credentials can be supplied in the URL.

**F5 / Send F5** sends asynchronously. The UI stays responsive; Esc cancels an in-flight request. Responses show status, duration, size, pretty-printed JSON or text, and a separate Headers tab. HTTP errors such as 400/500 and redirects remain inspectable. Redirects are not automatically followed. Requests time out after 30 seconds with a 10-second connection timeout. TLS certificate verification is enabled.

Responses retain up to **2 MiB** after decompression and label truncated previews. Non-UTF-8 text uses replacement characters. The response viewer supports vertical and horizontal scrolling. **History** / F8 shows the last 50 completed attempts, including network failures. Enter or click restores an attempt as a new request. Response bodies are not persisted in history.

## Keyboard and mouse

| Control | Action |
| --- | --- |
| Tab / Shift+Tab | Collections → Method → URL → Request editor → Output, or reverse |
| Enter / Space | Activate focused controls |
| F2 / Ctrl+W | Workspace chooser |
| F3 | Edit this workspace's environment |
| Alt+1 / Alt+2 / Alt+3 / Alt+4 | Params / Headers / Body / Auth; focus its editor |
| F4 / Shift+F4 | Next / previous request editor tab |
| F5 / Ctrl+Enter | Send request |
| F6 | Switch response body / headers |
| F7 | Rename selected collection |
| F8 | Switch Collections / History |
| Ctrl+G | New collection |
| Ctrl+N / Ctrl+D | New / duplicate request |
| Ctrl+S | Save workspace |
| Ctrl+C / Ctrl+V | Copy / paste |
| Ctrl+A | Select all text in editor or output |
| Shift+arrows | Select output text |
| Ctrl+Z / Ctrl+Y | Undo / redo text edits |
| Arrows / PageUp / PageDown | Move through response text |
| Ctrl/Alt+arrows in response | Scroll faster |
| Home / End in response | First / last response line |
| Esc | Cancel request, close dialog, or focus tree |
| F1 | Help |
| Ctrl+Q | Quit, with an unsaved-change prompt when needed |

Click controls, tree nodes, tabs, or text fields. Click text to position its cursor; drag text to select it. The wheel scrolls the panel under the pointer; Shift+wheel scrolls horizontally. Ctrl/Alt+wheel scrolls faster. The terminal must forward mouse events. Some terminals intercept Ctrl+Enter; use F5 instead.

Ctrl+C copies selected text, or the whole focused text field when nothing is selected. In the tree it copies the selected folder name or request URL. In the response viewer it copies the selected text, or the complete loaded body/headers document when nothing is selected. Drag over output text or use Shift+arrows to select; Ctrl+A selects the full document. Body and Headers have separate selection and scroll state. The output is read-only. Ctrl+C never quits. Ctrl+V pastes into the active editor, replacing a selection; single-line fields strip line breaks. Bracketed terminal paste also works. Replacing selected text records deletion and insertion separately, so undoing a replacement takes two Ctrl+Z presses.

Desktop clipboard integration uses arboard on Linux (Wayland/X11), macOS, and Windows. If unavailable, an internal clipboard keeps Ctrl+C/V working within postrust. Set `POSTRUST_CLIPBOARD=internal` to force this mode. Terminal paste, often Ctrl+Shift+V, remains available in remote sessions.

## Local storage

New workspaces with automatic paths are stored under `~/.config/postrust/workspaces/` on Linux, respecting `XDG_CONFIG_HOME`, or the platform config directory elsewhere. The chooser remembers recently opened external paths. The previous `~/.config/postrust/workspace.json` is also discovered.

Ctrl+S saves requests, collections, the environment, and history. Saves use a temporary file and atomic replacement, with Unix mode **0600**. Invalid existing files are preserved. Workspace files contain plain text, including saved credentials. Process environment values are resolved only when sending, so `{{API_TOKEN}}` stays a placeholder in saved requests.

Postman collection import/export, multipart uploads, cookie management, OAuth flows, scripts/assertions, WebSocket, and binary downloads are not implemented.

Linux is verified with real terminal smoke tests. Clipboard integration supports macOS and Windows through arboard; native builds and terminal behavior on those platforms have not yet been verified.

## Validate

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --locked
cargo build --release --locked
python3 scripts/smoke_tui.py
python3 scripts/smoke_workspaces.py
```

Layout tests render controls at 72×26, 80×26, 100×30, 120×40, and 160×48, checking complete labels, aligned controls, and non-overlapping hit areas. Text screen captures are written to `target/ui-review/` for inspection.

Both smoke tests use real Unix pseudo-terminals and isolated config directories. The HTTP test uses a local server to verify sending, auth, body, params, clipboard, Unicode, history and persistence. The workspace test creates and reopens two workspaces, checks environment isolation, and manages collections and requests through mouse and keyboard events.

GitHub Actions runs these checks and verifies the declared Rust 1.88 minimum. The package can be verified locally with `cargo package --locked`.

## Code layout

- `src/main.rs`: CLI, terminal lifecycle, workspace/workbench routing.
- `src/launcher.rs`: workspace creation, opening, discovery and recent list.
- `src/app.rs`: collection tree, workbench, dialogs, mouse and asynchronous operations.
- `src/ui.rs`: shared controls, styling, text editing and cursor mapping.
- `src/output.rs`: read-only response selection, copying and scrolling.
- `src/model.rs`: workspace storage, collections, environment resolution and migration.
- `src/http.rs`: request building, transport and response limits.
- `src/clipboard.rs`: desktop clipboard and internal fallback.

Uses Ratatui, Crossterm, ratatui-textarea, reqwest/Tokio, serde, and arboard. Follows standard Cargo layout and Rustfmt/Clippy defaults, with layout and interaction conventions taken from the adjacent dbrust project.
