# Client / server contract

Status: **Phase 1 draft** — SSH tunnel launcher only.
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

- Server revision used for this draft: `a4ffbf9` on `feature/agent-panel`
  (update this pin when the shared-server changes are merged or the client
  starts consuming a different server revision).

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
- verify the SSH host key;
- keep the tunnel alive with SSH keepalives;
- reconnect after network loss or machine sleep;
- close only its own forward and local listener on disconnect.

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
- explicit handling of a changed host key: warn and refuse automatically.

## Required SSH server policy

The server account should be restricted to the forwarding needed for VMI:

- allow local forwarding to `127.0.0.1:13517`;
- disallow arbitrary forwarding where the server policy supports it;
- shell access is not required by the client.

The client must not request a remote shell, a PTY, or remote forwarding.

## Phase 1: no server API

There is no custom API between the client and the server. The contract is:

```text
SSH local forward + existing NiceGUI web application + browser
```

The client's own control panel is a loopback-only HTTP service on `127.0.0.1`
(`/api/*`, authorized by a token generated for each run). It is local process
plumbing, not part of the client/server contract, and must never be exposed on
a network interface.

The client must not scrape the GUI, inject credentials into the page, or
require server-side code changes.

## Versioning and compatibility

The client build records:

```json
{
  "client_protocol": 1,
  "server_contract": 1,
  "server_revision": "a4ffbf9"
}
```

A future client may display a warning when the server's documented contract
version differs. There is no server handshake in Phase 1; document any
compatibility assumption here before adding one.

## Phase 2 (not part of this contract yet)

If client-side model-provider egress becomes a requirement, it will need:

- an authenticated, per-account relay between the server Agent container and
  the client companion;
- a server-side proxy endpoint for the `pi` subprocess/container;
- explicit start/stop and failure semantics when the client companion is
  offline.

That is a separate protocol. Do not implement it in the Phase 1 tunnel client.
