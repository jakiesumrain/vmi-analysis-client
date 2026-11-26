# VMI Analysis Client

Launch VMI Analysis from a local checkout or connect to a shared server over SSH.
The client can start Agent from a saved model connection. [VMI Analysis server](https://github.com/jakiesumrain/vmi-analysis-gui)

## Run on Windows

Install Rust 1.90 or newer with the MSVC toolchain. From this checkout, run

```powershell
cd client
cargo build --release --locked
.\target\release\vmi-analysis-client.exe
```

The client opens its control panel in your default browser. Use the Windows
tray icon to reopen the panel, reconnect, disconnect, or quit. Closing the
browser tab leaves the tunnel running.

## Use VMI on this computer

Install `uv` and make it available to the client. Select **Local (this computer)**,
then choose the root folder of your VMI Analysis GUI checkout. To start Agent,
install the Pi version specified by that checkout and save your model settings
and key under **Agent connection**. The key goes in the OS keychain.

Choose **Open VMI**. The client prepares the checkout's Python environment,
launches the GUI, and opens it in your browser.

## Connect over SSH

1. Enter the SSH host and username. Choose password or key-file authentication.
2. Choose **Connect**. Sign in with your VMI account when the web UI opens.
3. Choose **Save profile** to reuse the connection details.

Select **Remember** to save the SSH password or key passphrase.

For automatic sign-in, enter your VMI username and password under **VMI account
and Agent**, then save the profile. Add model settings and an API key to
start or reopen the Agent. Passwords and API keys go in the OS keychain.
Update the profile after changing your VMI password.

In manual-only mode, the client signs you in and opens the Workbench.
You can also connect with your own SSH tunnel and sign in directly from the
browser. [Shared-server guide](https://github.com/jakiesumrain/vmi-analysis-gui/blob/main/docs/shared-server.md#connect-from-linux-or-windows)

## Troubleshooting and development

Run `vmi-analysis-client --diagnose` to check the data directory and Windows
tray access. If a recorded SSH host key changes, verify the new key with the
server administrator before updating the trust record.

- [Server contract](docs/server-contract.md)
- [Implementation notes and verification](HANDOFF.md)
- [Development checks and Git workflow](AGENTS.md)
