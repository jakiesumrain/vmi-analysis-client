# VMI Analysis Client

Desktop companion for the VMI Analysis server. This repository is separate
from the server application so the client can use a Rust/Tauri toolchain, its
own release cadence, and its own installers without changing the server build.

## Read first

- [Server contract](docs/server-contract.md) — the client/server boundary.
- [Handoff](HANDOFF.md) — current architecture, server decisions and next steps.
- Server source checkout: `D:\work\vmi-analysis-gui`
- Server shared-server guide:
  `D:\work\vmi-analysis-gui\docs\shared-server.md`
- Agent backend reference:
  `D:\work\vmi-analysis-gui\src\vmi_analysis\agent_backend.py`

When the server repository is not available at that path, use the project URL
and the pinned server revision in the contract.

## Phase 1 scope

The client will:

1. Authenticate to the server over SSH.
2. Create a local port forward from `127.0.0.1:<local-port>` to
   `127.0.0.1:13517` on the server.
3. Open the system browser at the forwarded URL.
4. Keep the tunnel alive, reconnect after sleep/network changes, and expose a
   tray menu with connect/reconnect/disconnect/open actions.
5. Store credentials in the OS keychain and verify SSH host keys.

Use Tauri 2 with Rust, `russh`, and the OS keychain. Keep the first version
limited to the system browser; do not embed the NiceGUI app in a webview yet.

## Boundary rules

- Do not expose the server's web port to the network. The SSH forward is the
  only client/server transport.
- Use local forwarding only. Do not enable arbitrary shell or remote
  forwarding through the client.
- The Agent runs on the server (or in a server-started container). Its model
  provider traffic is server-side by default. Do not add a browser `fetch()`
  relay for provider traffic.
- Client-side Agent egress is an optional Phase 2 feature and requires a
  server-side relay design. Keep it out of the Phase 1 app.
- Never commit API keys, SSH private keys, passwords or tunnel credentials.

## Planned layout

```text
src-tauri/          # Rust/Tauri application
  src/ssh.rs        # SSH connection and host-key policy
  src/tunnel.rs     # local port forwarding and keepalive
  src/secrets.rs    # keychain / SSH agent integration
  src/tray.rs       # connection status and actions
ui/                 # optional small connection window
docs/               # contract and compatibility notes
```

## Development checks

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- Manual tunnel test against a local VMI server before each release.

Keep the server contract version in the client build metadata, and update
[docs/server-contract.md](docs/server-contract.md) before changing any
protocol or tunnel behavior.
