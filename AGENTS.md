# VMI Analysis Client

Desktop companion for the VMI Analysis server
(<https://github.com/jakiesumrain/vmi-analysis-gui>). This repository
(<https://github.com/jakiesumrain/vmi-analysis-client>) is separate from the
server application so the client can use its own Rust toolchain, its own
release cadence, and its own installers without changing the server build.

## Repositories

Both repositories always live side by side in the same parent folder:

| Repository | Location | URL |
| --- | --- | --- |
| Client (this one) | `.` | <https://github.com/jakiesumrain/vmi-analysis-client> |
| Server | `../vmi-analysis-gui` | <https://github.com/jakiesumrain/vmi-analysis-gui> |

All paths in this repository are relative. Never record a drive letter, user
profile path or other machine-specific absolute path in tracked files.

## Read first

- [Server contract](docs/server-contract.md) — the client/server boundary.
- [Handoff](HANDOFF.md) — current architecture, server decisions and next steps.
- Server repository: <https://github.com/jakiesumrain/vmi-analysis-gui>,
  checked out as the sibling directory `../vmi-analysis-gui`.
- Server shared-server guide: `../vmi-analysis-gui/docs/shared-server.md`.
- Agent backend reference: `../vmi-analysis-gui/src/vmi_analysis/agent_backend.py`.

If the sibling checkout is missing, clone
<https://github.com/jakiesumrain/vmi-analysis-gui> next to this repository and
check out the server revision pinned in
[docs/server-contract.md](docs/server-contract.md).

## Phase 1 scope

The client will:

1. Authenticate to the server over SSH.
2. Create a local port forward from `127.0.0.1:<local-port>` to
   `127.0.0.1:13517` on the server.
3. Serve a control panel on `127.0.0.1` and open it, and the forwarded VMI page,
   in the system browser.
4. Keep the tunnel alive, reconnect after sleep/network changes, and expose
   connect, disconnect, reconnect and quit through the panel and the tray icon.
5. Store credentials in the OS keychain and verify SSH host keys.

The client is a local service plus a browser panel. Do not embed a webview:
there is no WebView2/Tauri window, and the VMI application is never embedded
either. The panel must bind loopback only and require a per-run token.

## Boundary rules

- Use the SSH forward for shared servers. Direct local profiles launch
  the single-user GUI from a checkout on this computer.
- The control panel is HTTP on `127.0.0.1` only, with a per-run bearer token.
  Never bind it to a network interface and never drop the token check.
- Use local forwarding only. Do not enable arbitrary shell or remote
  forwarding through the client.
- The Agent runs on the server (or in a server-started container). Its model
  provider traffic is server-side by default. Do not add a browser `fetch()`
  relay for provider traffic.
- Client-side Agent egress is an optional Phase 2 feature and requires a
  server-side relay design. Keep it out of the Phase 1 app.
- Never commit API keys, SSH private keys, passwords or tunnel credentials.

## Layout

```text
client/
  src/lib.rs        # process startup, session lifecycle, shared state
  src/server.rs     # loopback HTTP panel, JSON API, event stream
  src/config.rs     # profiles and their store
  src/secrets.rs    # OS-keychain password/passphrase storage
  src/ssh.rs        # SSH connection, auth, host-key policy
  src/tunnel.rs     # local listener, forwarding, reconnect supervisor
  src/tray.rs       # native notification-area icon and menu
  assets/vmi.ico    # tray icon
  tests/testdata/   # throwaway SSH host keys for the tests
ui/                 # static panel UI (HTML, CSS, JS; no bundler)
docs/               # contract and compatibility notes
```

## Development checks

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- Manual tunnel test against a local VMI server before each release.

Update [docs/server-contract.md](docs/server-contract.md) before changing any
protocol or tunnel behavior.

## Git workflow

Branch from `main` using `feature/<name>` or `fix/<name>`. The owner has
authorized agents to commit, push the branch, and open a PR. Do not merge the
PR or fast-forward local `main`; after opening a PR, wait for the owner to
review and merge. All changes reach `main` through PRs. Run CI only when
requested. Preserve unrelated edits and dataset settings.
If direct GitHub access fails, use `HTTPS_PROXY=http://127.0.0.1:7897`.
