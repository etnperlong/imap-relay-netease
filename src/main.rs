//! Plain-IMAP relay to imap.163.com. Terminate TLS at an external proxy.
//!
//! `IMAP_LISTEN` plain TCP address (default `127.0.0.1:143`);
//! `IMAP_UPSTREAM_HOST` upstream name/IP (default `imap.163.com`);
//! `IMAP_UPSTREAM_PORT` upstream implicit-TLS port (default `993`);
//! `IMAP_ID_PROFILE` RFC 2971 ID preset (see `identity.rs`).

use std::{env, io, net::SocketAddr, sync::Arc, time::Duration};

use imap_relay_netease::run as relay;
use tokio::{
    net::{TcpListener, TcpStream},
    time::{sleep, timeout},
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, pki_types::ServerName},
};

mod identity;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

fn main() -> io::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> io::Result<()> {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let listen = parse_listen(env_var("IMAP_LISTEN")?.as_deref())?;
    let host = env_var("IMAP_UPSTREAM_HOST")?.unwrap_or_else(|| "imap.163.com".into());
    let port = parse_port(env_var("IMAP_UPSTREAM_PORT")?.as_deref())?;
    let server_name = ServerName::try_from(host.clone())
        .map_err(|err| invalid(&format!("IMAP_UPSTREAM_HOST {host:?}: {err}")))?;
    let id_command = identity::from_env()?;
    let connector = TlsConnector::from(Arc::new(tls_config()?));

    let listener = TcpListener::bind(listen).await?;
    eprintln!("relay listening on {listen}, upstream {host}:{port}");

    loop {
        let (client, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(err) => {
                eprintln!("accept failed: {:?}", err.kind());
                // Covers fd exhaustion / ephemeral-port pressure; aborted waits retry at once.
                if !matches!(
                    err.kind(),
                    io::ErrorKind::ConnectionAborted | io::ErrorKind::Interrupted
                ) {
                    sleep(Duration::from_millis(100)).await;
                }
                continue;
            }
        };
        let upstream = (host.clone(), port);
        let connector = connector.clone();
        let server_name = server_name.clone();
        tokio::spawn(async move {
            if let Err(err) = handle(client, connector, server_name, upstream, id_command).await {
                eprintln!("{peer}: connection ended: {:?}", err.kind());
            }
        });
    }
}

async fn handle(
    client: TcpStream,
    connector: TlsConnector,
    server_name: ServerName<'static>,
    upstream: (String, u16),
    id_command: &'static str,
) -> io::Result<()> {
    client.set_nodelay(true)?;
    let upstream = timeout(CONNECT_TIMEOUT, async move {
        let tcp = TcpStream::connect(upstream).await?;
        tcp.set_nodelay(true)?;
        connector.connect(server_name, tcp).await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "upstream connect/TLS timed out"))??;
    relay(client, upstream, id_command).await
}

fn tls_config() -> io::Result<ClientConfig> {
    let loaded = rustls_native_certs::load_native_certs();
    for err in &loaded.errors {
        eprintln!("certificate store warning: {err}");
    }
    let mut roots = RootCertStore::empty();
    let (added, ignored) = roots.add_parsable_certificates(loaded.certs);
    if added == 0 {
        return Err(invalid(
            "no usable root certificates; set SSL_CERT_FILE or install a CA bundle",
        ));
    }
    if ignored > 0 {
        eprintln!("ignored {ignored} invalid root certificate(s)");
    }
    Ok(ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth())
}

fn parse_listen(raw: Option<&str>) -> io::Result<SocketAddr> {
    raw.unwrap_or("127.0.0.1:143")
        .parse::<SocketAddr>()
        .ok()
        .filter(|addr| addr.port() != 0)
        .ok_or_else(|| invalid("IMAP_LISTEN must be IP:PORT with port 1-65535"))
}

fn parse_port(raw: Option<&str>) -> io::Result<u16> {
    match raw {
        None => Ok(993),
        Some(raw) => raw
            .parse::<u16>()
            .ok()
            .filter(|&port| port != 0)
            .ok_or_else(|| invalid("IMAP_UPSTREAM_PORT must be 1-65535")),
    }
}

fn env_var(key: &str) -> io::Result<Option<String>> {
    match env::var(key) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(invalid(&format!("{key}: {err}"))),
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listen_defaults_and_validates() {
        assert_eq!(parse_listen(None).unwrap().to_string(), "127.0.0.1:143");
        assert_eq!(parse_listen(Some("0.0.0.0:1993")).unwrap().port(), 1993);
        assert!(parse_listen(Some("127.0.0.1:0")).is_err());
        assert!(parse_listen(Some("nonsense")).is_err());
    }

    #[test]
    fn port_defaults_and_validates() {
        assert_eq!(parse_port(None).unwrap(), 993);
        assert_eq!(parse_port(Some("143")).unwrap(), 143);
        assert!(parse_port(Some("0")).is_err());
        assert!(parse_port(Some("65536")).is_err());
        assert!(parse_port(Some("smtp")).is_err());
    }

    #[test]
    fn upstream_host_must_be_a_server_name() {
        ServerName::try_from("imap.163.com".to_owned()).unwrap();
        ServerName::try_from("127.0.0.1".to_owned()).unwrap();
        assert!(ServerName::try_from("bad host".to_owned()).is_err());
    }
}
