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
forward manually before opening the browser. This companion app bundles that
into one process:

```text
VMI Analysis Client (local service)
  -> SSH connection to the server
  -> local forward 127.0.0.1:<local> -> server 127.0.0.1:13517
  -> opens the VMI page in the system browser
```

It is a separate repository (<https://github.com/jakiesumrain/vmi-analysis-client>)
so the Rust toolchain, release signing and installers do not enter the Python
server project (<https://github.com/jakiesumrain/vmi-analysis-gui>).

## Architecture decision: local service plus browser panel

The first implementation used a Tauri window for the client's own form. On the
owner's machine that window could not start: WebView2 environment creation
failed with `E_UNEXPECTED` (`0x8000FFFF`). A bare Tauri hello-world app failed
the same way, so it was not application code. The installed WebView2 Runtime was
129.0.2792.65 (September 2024) while the Edge updater service `edgeupdate` is
set to **Disabled**, so the runtime has not been updated.

Rather than depend on that runtime at all, the client is now:

- a local Rust process that owns the SSH tunnel;
- a small HTTP panel on `127.0.0.1`, opened in the system browser;
- a per-run bearer token that authorizes every API call.

This also matches the original rule that the VMI application is never embedded
in a webview: now nothing is embedded, and Chrome/Edge/Firefox render the panel.
No WebView2 is required. The VMI page still opens in the same browser through
the tunnel.

## Current client decisions

- **Rust only**; no Tauri, no WebView2, no embedded browser.
- **Panel:** static HTML/CSS/JS served by the client; JSON API plus server-sent
  events for status and activity.
- **Security:** binds `127.0.0.1` only; constant-time token check; the static
  panel contains no secrets.
- **SSH:** `russh`; no shelling out to OpenSSH.
- **Credentials:** OS keychain; key file and password (no agent yet).
- **Host keys:** verify and persist; refuse a changed key.
- **Reconnect:** bounded automatic retries; manual connect from the panel.
- **Single instance:** `session.json` records port and token; launching the
  client again reopens the panel instead of starting a second tunnel.
- **Tray:** not implemented yet; reopening the panel means starting the client.

## Current server decisions that constrain the client

- The NiceGUI server listens on `127.0.0.1:13517`; the shared-server web port
  must remain closed to incoming connections.
- Users need two identities: an SSH account for the tunnel and a VMI account
  for the web login.
- The Agent runs on the server (or in a server-started Docker/Podman
  container). Its provider API traffic is server-side by default.
- Do **not** use a browser `fetch()` relay for provider traffic.
- Client-side Agent egress is a possible Phase 2 feature and needs a
  server-side relay; it is not part of this client.

## Implementation status (Phase 1)

Implemented:

- Rust client process with a loopback panel, JSON API and SSE updates.
- SSH connection with password and key-file authentication.
- Host keys recorded on first contact and refused when they change.
- Local listener with automatic port choice, direct-tcpip forwarding,
  keepalive and bounded reconnect.
- Keychain storage for passwords and key passphrases.
- Profiles, single-instance reopen, panel **Quit client**.

Verified:

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- `cargo test`: 18 tests, including an end-to-end test that starts an
  in-process SSH server, connects with the production client code, forwards
  bytes to an echo server, and checks host-key trust, changed-key refusal and
  bad credentials.
- The panel was rendered in headless Chrome against a running client: saved
  profile selected, fields populated, key fields hidden in password mode,
  unauthorized API calls rejected with 401, profile saved to disk, and the
  connect failure path reporting through `/api/status`.

## Open questions

1. Tray icon: add one (native, not WebView2) or keep "start the client again"?
2. SSH agent authentication in the first release?
3. Windows signing and installer format; Linux packaging.
4. Whether the panel should use HTTPS on loopback or keep HTTP plus token.

## Next steps

1. Run a real tunnel against a shared-server checkout.
2. Add a native tray icon and SSH agent authentication.
3. Add Windows signing and installers; test a Linux build.
4. Update `docs/server-contract.md` before changing any tunnel or protocol
   behavior.
