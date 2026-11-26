# VMI Analysis Client

Open a local VMI Analysis GUI or connect to a shared server over SSH.
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

Choose **Local (this computer)** and enter the port shown by the running
VMI GUI. Choose **Open VMI** to open it in your browser.

To start Agent with a saved key, launch the GUI from its checkout:

```powershell
$env:VMI_GUI_LOCAL_CLIENT = '1'
uv run vmi-analysis
```

In the client, open **Agent connection**, enter your model settings and key,
and choose **Save profile**. Then choose **Open VMI**. The key goes in the
OS keychain.

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
