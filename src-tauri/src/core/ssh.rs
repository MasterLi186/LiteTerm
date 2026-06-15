use std::io;
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use ssh2::{Channel, Session};

use crate::config::connections::{AuthMethod, ConnectionConfig};

/// Wrapper around an SSH connection (TCP stream + libssh2 session).
pub struct SshConnection {
    session: Session,
    _stream: TcpStream,
}

impl SshConnection {
    /// Establish a TCP connection and perform the SSH handshake.
    ///
    /// `timeout_secs` controls the TCP connect timeout.
    pub fn connect(config: &ConnectionConfig, timeout_secs: u64) -> io::Result<Self> {
        let addr = format!("{}:{}", config.host, config.port);
        let stream = TcpStream::connect_timeout(
            &addr
                .parse()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?,
            Duration::from_secs(timeout_secs),
        )?;
        stream.set_nodelay(true)?;

        let mut session = Session::new()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        session.set_tcp_stream(stream.try_clone()?);
        session
            .handshake()
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?;

        Ok(Self {
            session,
            _stream: stream,
        })
    }

    /// Authenticate using the method specified in the config.
    ///
    /// Tries methods in order: Agent, Keyring (password), Key.
    pub fn authenticate(&self, config: &ConnectionConfig, password: Option<&str>) -> io::Result<()> {
        match &config.auth {
            AuthMethod::Agent => {
                self.session
                    .userauth_agent(&config.user)
                    .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;
            }
            AuthMethod::Keyring => {
                let pw = password.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "password required for keyring auth")
                })?;
                self.session
                    .userauth_password(&config.user, pw)
                    .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;
            }
            AuthMethod::Key => {
                let key = shellexpand::tilde(&config.key_path);
                let key_path = Path::new(key.as_ref());
                self.session
                    .userauth_pubkey_file(&config.user, None, key_path, password)
                    .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;
            }
        }
        Ok(())
    }

    /// Return a reference to the underlying `ssh2::Session`.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Open a shell channel with a PTY.
    ///
    /// Requests an xterm-256color PTY of the given dimensions, then starts a
    /// shell on the channel.
    pub fn open_shell_channel(&self, cols: u32, rows: u32) -> io::Result<Channel> {
        let mut channel = self
            .session
            .channel_session()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        channel
            .request_pty("xterm-256color", None, Some((cols, rows, 0, 0)))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        channel
            .shell()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(channel)
    }

    /// Enable SSH keepalive at the given interval (in seconds).
    pub fn set_keepalive(&self, interval_secs: u32) {
        self.session.set_keepalive(true, interval_secs);
    }
}
