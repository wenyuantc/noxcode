use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Handler, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub(crate) struct TestServerOpts {
    pub password_user: String,
    pub password: String,
    pub authorized_public_key: Option<PublicKey>,
}

impl Default for TestServerOpts {
    fn default() -> Self {
        Self {
            password_user: "tester".to_string(),
            password: "secret".to_string(),
            authorized_public_key: None,
        }
    }
}

pub(crate) struct TestSshServer {
    pub port: u16,
    #[allow(dead_code)]
    pub host_key: PublicKey,
    pub connections: Arc<AtomicUsize>,
    shutdown: russh::server::RunningServerHandle,
}

impl Drop for TestSshServer {
    fn drop(&mut self) {
        self.shutdown.shutdown("test server stopped".into());
    }
}

#[derive(Clone)]
struct Factory {
    connections: Arc<AtomicUsize>,
    password_user: String,
    password: String,
    authorized_public_key: Option<PublicKey>,
}

struct TestHandler {
    password_user: String,
    password: String,
    authorized_public_key: Option<PublicKey>,
    cat_channels: HashSet<ChannelId>,
}

impl russh::server::Server for Factory {
    type Handler = TestHandler;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        self.connections.fetch_add(1, Ordering::SeqCst);
        TestHandler {
            password_user: self.password_user.clone(),
            password: self.password.clone(),
            authorized_public_key: self.authorized_public_key.clone(),
            cat_channels: HashSet::new(),
        }
    }
}

impl Handler for TestHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if user == self.password_user && password == self.password {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        if self
            .authorized_public_key
            .as_ref()
            .is_some_and(|expected| expected == key)
        {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        let command = String::from_utf8_lossy(data);
        if command.starts_with("sh -lc") {
            session.data(
                channel,
                b"ok\nLinux testhost\n/home/tester\ngit version 2.39.5\n".to_vec(),
            )?;
            finish_channel(session, channel, 0)?;
            return Ok(());
        }
        if command == "hang" {
            return Ok(());
        }
        if command == "disconnect" {
            return Err(russh::Error::Disconnect);
        }
        if command == "cat" {
            self.cat_channels.insert(channel);
            return Ok(());
        }
        if let Some(text) = command.strip_prefix("echo ") {
            session.data(channel, text.as_bytes().to_vec())?;
            finish_channel(session, channel, 0)?;
            return Ok(());
        }
        if let Some(text) = command.strip_prefix("stderr ") {
            session.extended_data(channel, 1, text.as_bytes().to_vec())?;
            finish_channel(session, channel, 1)?;
            return Ok(());
        }
        if let Some(code) = command.strip_prefix("exit ") {
            let status = code.trim().parse::<u32>().unwrap_or(1);
            finish_channel(session, channel, status)?;
            return Ok(());
        }
        session.data(channel, format!("unknown:{command}").into_bytes())?;
        finish_channel(session, channel, 127)?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.cat_channels.contains(&channel) {
            session.data(channel, data.to_vec())?;
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.cat_channels.remove(&channel) {
            finish_channel(session, channel, 0)?;
        }
        Ok(())
    }
}

fn finish_channel(
    session: &mut Session,
    channel: ChannelId,
    status: u32,
) -> Result<(), russh::Error> {
    session.exit_status_request(channel, status)?;
    session.eof(channel)?;
    session.close(channel)?;
    Ok(())
}

impl TestSshServer {
    pub(crate) async fn start(opts: TestServerOpts) -> Self {
        let host_private =
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
        let host_key = host_private.public_key().clone();
        let connections = Arc::new(AtomicUsize::new(0));
        let config = russh::server::Config {
            keys: vec![host_private],
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            inactivity_timeout: None,
            ..Default::default()
        };
        let config = Arc::new(config);
        let factory = Factory {
            connections: connections.clone(),
            password_user: opts.password_user,
            password: opts.password,
            authorized_public_key: opts.authorized_public_key,
        };

        let (port_tx, port_rx) = oneshot::channel();
        let (handle_tx, handle_rx) = oneshot::channel();
        tokio::spawn(async move {
            let socket = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind test ssh server");
            let port = socket.local_addr().expect("local addr").port();
            let _ = port_tx.send(port);
            let mut factory = factory;
            let running = factory.run_on_socket(config, &socket);
            let _ = handle_tx.send(running.handle());
            let _ = running.await;
        });

        Self {
            port: port_rx.await.expect("test server port"),
            host_key,
            connections,
            shutdown: handle_rx.await.expect("test server handle"),
        }
    }
}
