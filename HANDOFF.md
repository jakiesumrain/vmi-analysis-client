# Handoff — VMI Analysis Client

Date: 2026-11-21

| Repository | Location | URL |
| --- | --- | --- |
| Client (this one) | `.` | <https://github.com/jakiesumrain/vmi-analysis-client> |
| Server | `../vmi-analysis-gui` | <https://github.com/jakiesumrain/vmi-analysis-gui> |

Server revision at handoff: `a4ffbf9` on `feature/agent-panel`
Shared-server docs: `../vmi-analysis-gui/docs/shared-server.md`

## Why this repo exists

The current shared-server workflow requires each user to start an SSH local
forward manually before opening the browser. This companion app will bundle
that into one desktop application:

```text
Tauri/Rust client
  -> SSH connection to the server
  -> local forward 127.0.0.1:<local> -> server 127.0.0.1:13517
  -> system browser at the forwarded URL
```

It is a separate repository (<https://github.com/jakiesumrain/vmi-analysis-client>)
so the Rust/Tauri toolchain, release signing and installers do not enter the
Python server project (<https://github.com/jakiesumrain/vmi-analysis-gui>).

## Current client decisions

- **Tauri 2 + Rust.**
- **SSH:** `russh`; do not shell out to OpenSSH in the first version.
- **Scope:** system browser, not an embedded webview.
- **Tray:** connected/reconnect/open/disconnect, with tunnel status.
- **Credentials:** OS keychain; SSH agent/key first, password fallback.
- **Host keys:** verify and persist known hosts; refuse automatic changed-key
  acceptance.
- **Phase 1:** tunnel only. No provider/model traffic from the client.

## Current server decisions that constrain the client

- The NiceGUI server listens on `127.0.0.1:13517`; the shared-server web port
  must remain closed to incoming connections.
- Users need two identities: an SSH account for the tunnel and a VMI account
  for the web login.
- The Agent runs on the server (or in a server-started Docker/Podman
  container). Its provider API traffic is server-side by default; the API key
  is passed only through the child environment and is redacted/cleared by the
  server.
- Do **not** use a browser `fetch()` relay for provider traffic: CORS,
  streaming, browser lifetime and provider rules make it fragile.
- Client-side Agent egress is a possible Phase 2 feature. It requires a
  server-side authenticated relay and container networking work; it is not part
  of this client's MVP.

## Open questions

1. SSH key UX: key file, SSH agent, or both in the first release?
2. Password auth: support via keychain, or defer?
3. Windows and Linux packaging targets and signing approach.
4. Whether to show a server/version handshake in the first release.
5. If Phase 2 egress is required, define the relay protocol before writing
   client code.

## Implementation status (Phase 1)

Implemented:

- Rust/Tauri 2 app with a window, tray menu and saved profiles.
- `russh` SSH connection with password and key-file authentication.
- Host keys recorded on first contact and refused when they change.
- Local loopback listener with automatic port choice, direct-tcpip
  forwarding, keepalive and bounded reconnect.
- Keychain storage for passwords and key passphrases.
- Static window UI with connection form, status, activity log and browser
  launch.

Verified in this checkout:

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- `cargo test`: 15 tests, including an end-to-end test that starts an
  in-process SSH server, connects with the real client code, forwards bytes to
  an echo server, and checks host-key trust, changed-key refusal and bad
  credentials.

Not verified here: the window could not be created in this automation
environment because WebView2 environment creation fails there (`E_UNEXPECTED`)
even for a bare Tauri app. Run the app once on a normal desktop session to
confirm the window and tray.

## Next steps

1. Run the app on a desktop session and connect to a real shared-server
   checkout.
2. Add SSH agent authentication.
3. Add Windows signing and installers, then a Linux build.
4. Update `docs/server-contract.md` before changing any tunnel or protocol
   behavior.
