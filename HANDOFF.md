# Client implementation notes

The VMI Analysis Client is a Rust process that manages an SSH tunnel and
serves a local control panel. It binds both listeners to `127.0.0.1`. The
browser renders the control panel and the VMI workbench. The original Tauri
window was replaced after WebView2 startup failed on the test machine,
including in a minimal Tauri app.

The shared Linux server runs the Workbench and Agent. The client uses the
regular VMI login and can request Agent startup through the authenticated
session endpoint. Direct browser access through an SSH tunnel remains
available. [Server contract](docs/server-contract.md)

## Client data

The application config directory on Windows is
`%APPDATA%\com.vmi.analysis.client\`. Set `VMI_CLIENT_CONFIG_DIR` to choose
another directory. When the default is unwritable, the client tries a
`data` directory beside the executable, then a temporary directory.
The startup log and `--diagnose` show the chosen location.

| File | Contents |
| --- | --- |
| `profiles.json` | SSH, VMI account, and Agent settings |
| `known_hosts` | Recorded SSH host keys in OpenSSH format |
| `session.json` | Current control-panel port and per-run token |

The OS keychain stores saved SSH credentials, VMI passwords, and model API keys.
The server receives the model key when starting the Agent and clears its session
copies at shutdown.

## Verification

- On Windows, the real client opened a running VMI server through an SSH
  fixture. The browser loaded the Agent and Workbench, and reconnect reused
  the same local port.
- The Windows tray icon appeared in Explorer's overflow. Its **Disconnect**,
  **Reconnect**, and **Quit client** actions worked on the interactive desktop.
- After a client restart, the saved VMI password signed the browser in and the
  saved model API key reached the Agent startup path. The test used a stub
  for startup. Ordinary browser sign-in passed in the same test.
- Rust tests cover SSH forwarding, changed host keys, authentication failure,
  remote channel denial, and reconnect with an active forwarded connection.

## Remaining work

- SSH agent authentication. The client currently offers password and key-file
  authentication.
- Verify Linux packaging and add trays for Linux and macOS.
- Windows installer signing and automatic updates.
