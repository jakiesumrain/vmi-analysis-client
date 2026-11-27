# Client / server contract

Status: SSH tunnel launcher and direct local desktop connection.
Protocol version: **1**

This contract covers the VMI Analysis client
(<https://github.com/jakiesumrain/vmi-analysis-client>) and the VMI Analysis
server (<https://github.com/jakiesumrain/vmi-analysis-gui>). Both repositories
are checked out side by side in the same parent folder, so every server path
below is relative to this client repository.

## Reference server

| Repository | Location | URL |
| --- | --- | --- |
| Client (this one) | `.` | <https://github.com/jakiesumrain/vmi-analysis-client> |
| Server | `../vmi-analysis-gui` | <https://github.com/jakiesumrain/vmi-analysis-gui> |

- Agent startup via `POST /api/agent/session` requires server commit
  `69b968e` or a later revision containing that change.
  [Server change](https://github.com/jakiesumrain/vmi-analysis-gui/pull/95)

## Network shape

The server runs the NiceGUI web application on loopback:

```text
server 127.0.0.1:13517
```

The server web port must not be exposed to incoming network connections.
The client reaches it through SSH local port forwarding:

```text
client 127.0.0.1:<local-port>  ==SSH==>  server 127.0.0.1:13517
```

The client opens the system browser at:

```text
http://127.0.0.1:<local-port>
```

The client must:

- choose a free loopback port, or honour an explicitly configured one;
- fail clearly if the remote forward cannot be established;
- confirm that SSH can open a channel to the server's web endpoint before
  reporting a connected tunnel, including after reconnect;
- verify the SSH host key;
- keep the tunnel alive with SSH keepalives;
- reconnect after network loss or machine sleep;
- close only its own forward and local listener on disconnect.
  Manual reconnect waits for that listener to close before binding its port again.

Equivalent manual command for reference:

```sh
ssh -N -o ExitOnForwardFailure=yes \
    -L 127.0.0.1:13517:127.0.0.1:13517 \
    ssh-user@linux-host
```

## Authentication

Two separate authentication steps exist:

1. **SSH authentication** — SSH account and key or password, plus host-key
   verification. This protects the tunnel.
2. **VMI authentication** — the NiceGUI sign-in page selects the person's
   workspace. It uses the server's VMI account database, not the SSH identity.

The client must not treat SSH success as VMI success. The browser handles the
VMI login.

The client should support:

- SSH agent and key-file authentication first;
- password fallback through the OS keychain, not plaintext storage;
- SSH host-key verification against a known-hosts store;
- explicit handling of changed keys and unreadable trust records: refuse the
  connection before authentication, including a different key algorithm for
  a previously recorded host.

## Required SSH server policy

The server account should be restricted to the forwarding needed for VMI:

- allow local forwarding to `127.0.0.1:13517`;
- disallow arbitrary forwarding where the server policy supports it;
- shell access is not required by the client.

The client must not request a remote shell, a PTY, or remote forwarding.

## Browser-only connection

The GUI works through a plain SSH forward with ordinary browser sign-in:

```text
SSH local forward + existing NiceGUI web application + browser
```

The client's own control panel is a loopback-only HTTP service on `127.0.0.1`
(`/api/*`, authorized by a token generated for each run). It is local process
plumbing, not part of the client/server contract, and must never be exposed on
a network interface.

The client must not scrape the GUI, inject credentials into the page, or
require desktop sign-in for regular tunneling.

## Optional desktop sign-in

The client can save the VMI account password and the model API key in the
OS keychain, scoped to the saved server and account. The model key also
belongs to the exact provider URL. Connection preferences and model IDs
live in the profile.

The client submits the saved account to VMI's normal `POST /login` over
the active SSH tunnel. If the profile has model settings and a key, the
client calls the authenticated `POST /api/agent/session` with that session.
Current servers register the account's provider connection. The first
message starts or resumes a chat. Older servers start Agent immediately.
Manual-only servers refuse the Agent request.

The local control panel receives the normal HttpOnly VMI session cookie and
then opens the forwarded VMI page in that browser. The password and model API
key stay out of browser navigation and GUI form fields. Both loopback
listeners use the same browser hostname, so the cookie applies across ports.

## Direct local launch

A local profile points to a VMI Analysis GUI checkout on this computer.
The client uses `uv` to prepare the checkout's Python environment, then
starts the existing GUI on loopback and opens it in the browser. Local
mode requires no SSH connection or VMI account.

The client scopes the saved model key to the checkout path and provider
URL in the OS keychain. When starting the GUI, it supplies the connection
to a client-owned startup script in the new Python process. The script
consumes the credentials from the process environment and connects the
provider through the checkout's Agent backend. The first message starts or
resumes a chat. Older checkouts still start Agent at launch.

## Versioning and compatibility

The client uses the existing browser login endpoint and connects the Agent
provider only when the profile includes model settings and a saved key.
If the server lacks the Agent endpoint or runs in manual-only mode, the
client still opens the signed-in GUI.

## Phase 2 (not part of this contract yet)

If client-side model-provider egress becomes a requirement, it will need:

- an authenticated, per-account relay between the server Agent container and
  the client companion;
- a server-side proxy endpoint for the `pi` subprocess/container;
- explicit start/stop and failure semantics when the client companion is
  offline.

That is a separate protocol. Do not implement it in the Phase 1 tunnel client.
