# Changelog

## 0.1.0 — 2026-11-11

Initial release of postrust, a terminal HTTP workbench inspired by ATAC and dbrust.

- Create and open independent workspaces, with recent workspace discovery.
- Organize requests in expandable collection trees; create, rename and delete collections and move requests between them.
- Keep one environment per workspace, with `{{variable}}` substitution and process environment fallback.
- Send HTTP(S) requests asynchronously with query parameters, headers, raw bodies and bearer authentication.
- Inspect response status, timing, size, headers and formatted JSON or text, with a bounded 2 MiB preview.
- Navigate using the mouse, Tab and arrows; switch request tabs with Alt+1–4 or F4. Toolbar controls have aligned borders and symmetric label padding.
- Select and copy response text; use Ctrl+C/Ctrl+V with desktop or internal clipboard support.
- Save request history and workspace data with private Unix file permissions; migrate older workspace files with a backup.

The initial release does not include Postman import/export, multipart uploads, cookie management, OAuth flows, scripts, WebSocket or binary downloads. See README.md for controls and platform notes.
