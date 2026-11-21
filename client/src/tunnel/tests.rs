//! End-to-end tunnel tests against an in-process SSH server.
//!
//! The test server accepts the password `secret` for the user `tester`, allows
//! direct-tcpip forwarding, and relays each channel to the requested local
//! address. This exercises the real client code path: host-key checking,
//! authentication, channel opening and bidirectional copying.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use russh::keys::PrivateKey;
use russh::server::{
    Auth, ChannelOpenHandle, Config as ServerConfig, Msg, Server, Session as ServerSession,
};
use russh::Channel;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;
use crate::ssh::{self, AuthMaterial, ConnectSpec, HostKeyOutcome, SshErrorKind};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

const HOST_KEY: &str = include_str!("../../tests/testdata/host_key");
const OTHER_PUBLIC_KEY: &str = include_str!("../../tests/testdata/host_key2.pub");

struct TestServer;

impl Server for TestServer {
    type Handler = TestHandler;

    fn new_client(&mut self, _peer: Option<SocketAddr>) -> TestHandler {
        TestHandler
    }
}

struct TestHandler;

impl russh::server::Handler for TestHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if user == "tester" && password == "secret" {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        let host = host_to_connect.to_owned();
        tokio::spawn(async move {
            let Ok(mut target) = TcpStream::connect((host.as_str(), port_to_connect as u16)).await
            else {
                return;
            };
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut target, &mut stream).await;
        });
        Ok(())
    }
}

async fn start_ssh_server() -> u16 {
    let key = PrivateKey::from_openssh(HOST_KEY).expect("test host key");
    let config = Arc::new(ServerConfig {
        keys: vec![key],
        ..Default::default()
    });
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ssh server");
    let port = listener.local_addr().expect("ssh address").port();
    let mut server = TestServer;
    tokio::spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });
    wait_until_listening(port).await;
    port
}

async fn wait_until_listening(port: u16) {
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("SSH test server did not start listening on port {port}");
}

async fn start_echo_server() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind echo server");
    let port = listener.local_addr().expect("echo address").port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0u8; 1024];
                loop {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if stream.write_all(&buffer[..read]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });
    port
}

fn spec(port: u16) -> ConnectSpec {
    ConnectSpec {
        host: "127.0.0.1".into(),
        port,
        user: "tester".into(),
        auth: AuthMaterial::Password("secret".into()),
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("vmi-client-{label}-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[tokio::test]
async fn forwards_tcp_through_the_ssh_session() {
    let dir = temp_dir("forward");
    let known_hosts = dir.join("known_hosts");
    let ssh_port = start_ssh_server().await;
    let echo_port = start_echo_server().await;

    let (handle, outcome) = ssh::connect(&spec(ssh_port), &known_hosts)
        .await
        .expect("ssh connect");
    assert_eq!(outcome, HostKeyOutcome::Learned);
    assert!(known_hosts.exists(), "host key should be recorded");

    let session: Session = Arc::new(tokio::sync::Mutex::new(Some(handle)));
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind local listener");
    let local = listener.local_addr().expect("local address");
    let target = Arc::new(Target {
        remote_host: "127.0.0.1".into(),
        remote_port: echo_port,
    });

    let session_for_task = Arc::clone(&session);
    tokio::spawn(async move {
        let (tcp, peer) = listener.accept().await.expect("accept local connection");
        forward(tcp, peer, session_for_task, target).await;
    });

    let mut client = TcpStream::connect(local).await.expect("connect to tunnel");
    client
        .write_all(b"ping")
        .await
        .expect("write through tunnel");
    let mut buffer = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(10), client.read_exact(&mut buffer))
        .await
        .expect("echo timed out")
        .expect("read echo");
    assert_eq!(&buffer, b"ping");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn second_connection_trusts_the_recorded_host_key() {
    let dir = temp_dir("trusted");
    let known_hosts = dir.join("known_hosts");
    let ssh_port = start_ssh_server().await;

    let (first, outcome) = ssh::connect(&spec(ssh_port), &known_hosts)
        .await
        .expect("first connect");
    assert_eq!(outcome, HostKeyOutcome::Learned);
    drop(first);

    let (_, outcome) = ssh::connect(&spec(ssh_port), &known_hosts)
        .await
        .expect("second connect");
    assert_eq!(outcome, HostKeyOutcome::Trusted);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn changed_host_key_is_refused() {
    let dir = temp_dir("changed");
    let known_hosts = dir.join("known_hosts");
    let ssh_port = start_ssh_server().await;

    let (first, _) = ssh::connect(&spec(ssh_port), &known_hosts)
        .await
        .expect("first connect");
    drop(first);

    // Pretend the recorded key belonged to a different server.
    let other = OTHER_PUBLIC_KEY.trim();
    std::fs::write(&known_hosts, format!("[127.0.0.1]:{ssh_port} {other}\n"))
        .expect("write known_hosts");

    match ssh::connect(&spec(ssh_port), &known_hosts).await {
        Ok(_) => panic!("a changed host key must be refused"),
        Err(err) => {
            assert_eq!(err.kind, SshErrorKind::HostKeyChanged);
            assert!(err.message.contains("does not match"));
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wrong_password_is_rejected() {
    let dir = temp_dir("auth");
    let known_hosts = dir.join("known_hosts");
    let ssh_port = start_ssh_server().await;

    let mut bad = spec(ssh_port);
    bad.auth = AuthMaterial::Password("wrong".into());

    match ssh::connect(&bad, &known_hosts).await {
        Ok(_) => panic!("bad credentials must fail"),
        Err(err) => assert_eq!(err.kind, SshErrorKind::Authentication),
    }

    let _ = std::fs::remove_dir_all(&dir);
}
