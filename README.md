# VMI Analysis Client

Local launcher for the [VMI Analysis server](https://github.com/jakiesumrain/vmi-analysis-gui).
It opens the SSH tunnel that shared-server users would otherwise start by hand,
keeps it alive, and serves its own control panel on loopback. Both the panel and
the VMI Analysis application open in your normal browser — the client never
embeds a webview.

```text
vmi-analysis-client  (local process)
  ├─ control panel   http://127.0.0.1:<panel port>/?token=<run token>
  ├─ tunnel          http://127.0.0.1:<tunnel port> -> server 127.0.0.1:13517
  └─ opens the browser
```

## Features

- Password or key-file SSH authentication through `russh`.
- Host keys recorded on first contact; a changed key is refused.
- Automatic local port selection, or a fixed port you choose.
- Keepalive plus bounded automatic reconnect (5 attempts, 3 s apart).
- Passwords and key passphrases in the OS keychain, never on disk.
- Saved profiles, served by the panel and read back on the next launch.
- A second launch detects the running instance and reopens the panel instead of
  starting a second tunnel.
- A native tray icon: open the panel, reconnect, disconnect, quit; its tooltip
  shows the tunnel state. Double-click opens the panel.
- **Quit client** stops the tunnel and the process; closing the tab does not.

## Requirements

- Rust 1.90+ (MSVC toolchain on Windows).
- A browser. Chrome, Edge or Firefox all work; the default browser is opened.
- Windows 10/11. Linux needs the Secret Service for keychain storage and a
  running D-Bus session; that build is untested.

No WebView2 runtime is required: the client has no embedded browser.

## Build and run

```powershell
cd client
cargo build --release
.\target\release\vmi-analysis-client.exe
```

For development, `cargo run` in `client/` does the same and keeps console logs.

On first launch the client prints and opens a panel URL such as
`http://127.0.0.1:3070/?token=…`. The token is regenerated every run and is
required by every API call.

## Use

1. Enter the server host and your SSH user.
2. Choose password or key-file authentication.
3. Leave the local port blank to let the client pick a free one.
4. Choose **Connect**. When the tunnel is up, the VMI page opens in the browser
   (disable *Open the browser when the tunnel connects* to do it manually).
5. Sign in to VMI Analysis with your VMI account, not your SSH account.

Use **Save profile** to keep the connection settings. Passwords are stored only
when **Remember** is checked, and then only in the OS keychain.

### Remember a VMI account and Agent settings

In the client, open **VMI account and Agent**, enter your VMI account name and
password, then choose **Save profile**. Include model settings and an API key
if you want the client to start the Agent as well.

On later launches, select the profile and connect. The client opens the GUI in
the same browser tab and signs in using the saved account. The VMI password
and model API key live in the OS keychain. When you change your VMI password,
update the saved profile in the client.

## Files

Everything lives in the application-config directory, for example
`%APPDATA%\com.vmi.analysis.client\` on Windows:

| File | Contents |
| --- | --- |
| `profiles.json` | Saved profiles. Never contains passwords. |
| `known_hosts` | Trusted server host keys, OpenSSH format. |
| `session.json` | Port and token of the running instance, for reopening the panel. |

Set `VMI_CLIENT_CONFIG_DIR` to use another directory (portable installs or
locked-down profiles).

## Troubleshooting

Run `vmi-analysis-client --diagnose` to print where the client stores its data,
the Windows session, window station and desktop, and whether Explorer's tray
window is reachable.

If the default config folder is not writable, the client falls back to a `data`
folder beside the executable, then to a temp folder. The chosen location is
reported at startup. A tray icon can only appear when the process runs on the
interactive desktop; some security products also block unsigned programs from
adding one.

## Security model

- The panel and the tunnel both bind `127.0.0.1` only. Nothing listens on a
  network interface.
- Every API call needs the per-run bearer token; the token is compared in
  constant time. The static panel itself contains no secrets.
- Only local forwarding is used: no shell, no PTY, no remote forwarding.
- A changed host key stops the connection. Delete the `known_hosts` entry only
  after verifying the change with the server owner.
- Passwords and passphrases live in the OS keychain, never in `profiles.json`.
- Model-provider traffic from the Agent stays server-side; this client has no
  egress relay.

## Repository layout

```text
client/
  src/lib.rs        process startup, session lifecycle, shared state
  src/server.rs     loopback HTTP panel, JSON API, event stream
  src/ssh.rs        SSH connection, auth, host-key policy
  src/tunnel.rs     listener, forwarding, reconnect supervisor
  src/tray.rs       native notification-area icon and menu
  src/secrets.rs    OS-keychain storage
  src/config.rs     profiles and their store
  assets/vmi.ico    tray icon
  tests/testdata/   throwaway SSH host keys for the tests
ui/                 panel served by the client (HTML, CSS, JS; no bundler)
docs/               client/server contract
```

## Development checks

```powershell
cd client
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`cargo test` includes an end-to-end tunnel test: it starts an in-process SSH
server, connects with the production client code, forwards bytes to an echo
server, and checks host-key trust, changed-key refusal and bad credentials.

The client/server contract is in [docs/server-contract.md](docs/server-contract.md).
Git workflow rules are in [AGENTS.md](AGENTS.md).

## Status

Phase 1: SSH tunnel, loopback control panel and a native tray icon. Not yet
implemented:

- SSH agent authentication (key file and password only).
- Tray menus on Linux and macOS (Windows only today).
- Signed installers and automatic updates.
- Client-side Agent egress (Phase 2, needs a server-side relay).




