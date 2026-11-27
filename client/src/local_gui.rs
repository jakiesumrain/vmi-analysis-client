//! Launch a trusted VMI Analysis checkout with the user's saved connection.

use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::desktop::AgentProfile;

const BOOTSTRAP: &str = include_str!("local_gui.py");

pub(crate) struct LocalGui {
    pub checkout: PathBuf,
    pub model: Option<AgentProfile>,
    port: u16,
    child: Child,
}

impl LocalGui {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn is_running(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn resolve_python(checkout: &Path) -> Result<PathBuf, String> {
    let output = command("uv")
        .current_dir(checkout)
        .args([
            "run",
            "--locked",
            "python",
            "-c",
            "import sys; sys.stdout.buffer.write(sys.executable.encode('utf-8'))",
        ])
        .output()
        .map_err(|_| "Could not find uv. Install uv and make it available to the VMI client.")?;
    if !output.status.success() {
        return Err("Could not prepare the VMI checkout. Run uv sync --locked in that folder to see the error.".into());
    }
    let executable = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if !executable.is_absolute() || !executable.is_file() {
        return Err("Could not find the Python executable for this VMI checkout.".into());
    }
    Ok(executable)
}

pub(crate) fn launch(
    checkout: PathBuf,
    model: Option<AgentProfile>,
    connection: Option<Value>,
) -> Result<LocalGui, String> {
    let python = resolve_python(&checkout)?;
    let port = {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "Could not reserve a local VMI port.")?;
        listener
            .local_addr()
            .map_err(|_| "Could not read the local VMI port.")?
            .port()
    };
    let mut command = command(python);
    command
        .current_dir(&checkout)
        .args(["-c", BOOTSTRAP])
        .env_remove("VMI_SHARED_CONFIG")
        .env("VMI_GUI_NO_SHOW", "1")
        .env("VMI_GUI_PORT", port.to_string())
        .stdout(Stdio::null());
    if let Some(connection) = connection {
        command.env("VMI_CLIENT_AGENT_CONNECTION", connection.to_string());
    } else {
        command.env_remove("VMI_CLIENT_AGENT_CONNECTION");
    }
    let child = command
        .spawn()
        .map_err(|_| "Could not start Python for the VMI checkout.")?;
    drop(command);
    let mut gui = LocalGui {
        checkout,
        model,
        port,
        child,
    };
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        match gui.child.try_wait() {
            Ok(Some(status)) if status.code() == Some(78) => {
                return Err("Could not set up the local Agent provider. Check the saved model settings.".into());
            }
            Ok(Some(_)) => return Err("VMI exited before opening the browser. Run uv run vmi-analysis in the checkout to see the error.".into()),
            Err(_) => break,
            Ok(None) => {}
        }
        if TcpStream::connect_timeout(
            &(Ipv4Addr::LOCALHOST, port).into(),
            Duration::from_millis(150),
        )
        .is_ok()
        {
            return Ok(gui);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = gui.child.kill();
    let _ = gui.child.wait();
    Err("The local VMI GUI did not start in time.".into())
}
