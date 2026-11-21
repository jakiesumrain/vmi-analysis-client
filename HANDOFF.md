# Handoff — VMI Analysis Client

Date: 2026-11-21
Server checkout: `D:\work\vmi-analysis-gui`
Server revision at handoff: `a4ffbf9` on `feature/agent-panel`
Shared-server docs: `docs/shared-server.md` in that checkout

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

It is a separate repository so the Rust/Tauri toolchain, release signing and
installers do not enter the Python server project.

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

## Next steps

1. Scaffold the Tauri 2 app and tray shell.
2. Implement SSH auth, host-key verification and local forwarding.
3. Add reconnect/keepalive and local-port conflict handling.
4. Test against a local shared-server checkout using the manual command in
   `docs/server-contract.md`.
5. Add Windows and Linux installers after the tunnel MVP works.
