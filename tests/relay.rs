//! Wire-level tests: `run` drives one duplex end against a scripted IMAP server.

use std::io;
use std::time::Duration;

use tokio::io::{
    AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf,
    duplex, split,
};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use imap_relay_netease::run;

const STEP: Duration = Duration::from_secs(1);
const QUIET: Duration = Duration::from_millis(200);

/// Untagged, terminator-free ID command handed to `run`.
const ID: &str = r#"ID ("name" "Test archiver")"#;

type Reader = BufReader<ReadHalf<DuplexStream>>;

struct Wire {
    /// What the client receives.
    client_in: Reader,
    /// What the test client writes.
    client_out: WriteHalf<DuplexStream>,
    /// What the server receives.
    upstream_in: Reader,
    /// What the test server writes.
    upstream_out: WriteHalf<DuplexStream>,
    _relay: JoinHandle<io::Result<()>>,
}

impl Wire {
    fn new() -> Self {
        let (client, relay_client) = duplex(64 * 1024);
        let (relay_upstream, upstream) = duplex(64 * 1024);
        let relay = tokio::spawn(run(relay_client, relay_upstream, ID));
        let (client_in, client_out) = split(client);
        let (upstream_in, upstream_out) = split(upstream);
        Self {
            client_in: BufReader::new(client_in),
            client_out,
            upstream_in: BufReader::new(upstream_in),
            upstream_out,
            _relay: relay,
        }
    }

    async fn client_line(&mut self) -> String {
        line(&mut self.client_in).await
    }

    async fn upstream_line(&mut self) -> String {
        line(&mut self.upstream_in).await
    }

    async fn client_send(&mut self, bytes: &[u8]) {
        timeout(STEP, self.client_out.write_all(bytes))
            .await
            .expect("relay stalled on a client write")
            .expect("client write failed");
    }

    async fn upstream_send(&mut self, bytes: &[u8]) {
        timeout(STEP, self.upstream_out.write_all(bytes))
            .await
            .expect("relay stalled on an upstream write")
            .expect("upstream write failed");
    }

    /// Reads the injected ID command, checks it carries the preset verbatim, and returns its tag.
    async fn expect_id(&mut self) -> String {
        let command = self.upstream_line().await;
        let body = command.trim_end_matches(['\r', '\n']);
        let (tag, rest) = body.split_once(' ').expect("ID command has a tag");
        assert_eq!(rest, ID, "injected ID command differs from the preset");
        tag.to_string()
    }

    /// Fails if the server receives anything within the quiet window.
    async fn expect_quiet(&mut self) {
        assert!(
            timeout(QUIET, line(&mut self.upstream_in)).await.is_err(),
            "relay sent a command upstream while it had no reason to"
        );
    }
}

async fn line(reader: &mut Reader) -> String {
    let mut line = String::new();
    timeout(STEP, reader.read_line(&mut line))
        .await
        .expect("relay did not respond in time")
        .expect("read failed");
    line
}

#[tokio::test]
async fn preauth_injects_id_immediately_and_tolerates_bad_id() {
    let mut w = Wire::new();
    w.upstream_send(b"* PREAUTH [CAPABILITY IMAP4rev1] ready\r\n")
        .await;
    assert_eq!(
        w.client_line().await,
        "* PREAUTH [CAPABILITY IMAP4rev1] ready\r\n"
    );

    // PREAUTH needs no client command before the ID exchange starts.
    let tag = w.expect_id().await;
    // A refused injection must not break the already-authenticated session.
    w.upstream_send(format!("{tag} BAD Unknown command\r\n").as_bytes())
        .await;

    w.client_send(b"aA1 CAPABILITY\r\n").await;
    assert_eq!(w.upstream_line().await, "aA1 CAPABILITY\r\n");
    w.upstream_send(b"* CAPABILITY IMAP4rev1\r\naA1 OK CAPABILITY completed\r\n")
        .await;
    // The injected exchange stays hidden and nothing else is disturbed.
    assert_eq!(w.client_line().await, "* CAPABILITY IMAP4rev1\r\n");
    assert_eq!(w.client_line().await, "aA1 OK CAPABILITY completed\r\n");
}

#[tokio::test]
async fn pipelined_select_is_embargoed_until_id_completes() {
    let mut w = Wire::new();
    w.upstream_send(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
        .await;
    assert_eq!(
        w.client_line().await,
        "* OK [CAPABILITY IMAP4rev1] ready\r\n"
    );

    // Mixed-case tags and a pipelined command: SELECT must not overtake the ID exchange.
    w.client_send(b"aB1 LOGIN user pass\r\naB2 SELECT INBOX\r\n")
        .await;
    assert_eq!(w.upstream_line().await, "aB1 LOGIN user pass\r\n");

    // Untagged traffic read ahead with the tagged OK still reaches the client.
    w.upstream_send(b"aB1 OK LOGIN completed\r\n* OK [ALERT] Notice\r\n")
        .await;
    assert_eq!(w.client_line().await, "* OK [ALERT] Notice\r\n");

    let tag = w.expect_id().await;
    w.expect_quiet().await;

    // Untagged traffic arriving while the ID exchange is open is not held back.
    w.upstream_send(b"* ID (\"name\" {6}\r\nServer)\r\n* OK Another notice\r\n")
        .await;
    assert_eq!(w.client_line().await, "* OK Another notice\r\n");

    w.upstream_send(format!("{tag} OK ID completed\r\n* OK After ID\r\n").as_bytes())
        .await;
    assert_eq!(w.client_line().await, "aB1 OK LOGIN completed\r\n");
    assert_eq!(w.client_line().await, "* OK After ID\r\n");
    assert_eq!(w.upstream_line().await, "aB2 SELECT INBOX\r\n");

    w.upstream_send(b"* 0 EXISTS\r\naB2 OK SELECT completed\r\n")
        .await;
    assert_eq!(w.client_line().await, "* 0 EXISTS\r\n");
    assert_eq!(w.client_line().await, "aB2 OK SELECT completed\r\n");
}

#[tokio::test]
async fn failed_login_retries_with_literal_arguments() {
    let mut w = Wire::new();
    w.upstream_send(b"* OK ready\r\n").await;
    assert_eq!(w.client_line().await, "* OK ready\r\n");

    w.client_send(b"aA1 LOGIN user wrong\r\n").await;
    assert_eq!(w.upstream_line().await, "aA1 LOGIN user wrong\r\n");
    w.upstream_send(b"aA1 NO [AUTHENTICATIONFAILED] invalid credentials\r\n")
        .await;
    assert_eq!(
        w.client_line().await,
        "aA1 NO [AUTHENTICATIONFAILED] invalid credentials\r\n"
    );
    // A failed login must not trigger an ID exchange.
    w.expect_quiet().await;

    // Retry with synchronized literals in both arguments.
    w.client_send(b"aA2 LOGIN {5}\r\n").await;
    assert_eq!(w.upstream_line().await, "aA2 LOGIN {5}\r\n");
    w.upstream_send(b"+ Ready for literal\r\n").await;
    assert_eq!(w.client_line().await, "+ Ready for literal\r\n");

    w.client_send(b"admin {6}\r\n").await;
    assert_eq!(w.upstream_line().await, "admin {6}\r\n");
    w.upstream_send(b"+ Ready for literal\r\n").await;
    assert_eq!(w.client_line().await, "+ Ready for literal\r\n");

    w.client_send(b"secret\r\n").await;
    assert_eq!(w.upstream_line().await, "secret\r\n");
    w.upstream_send(b"aA2 OK LOGIN completed\r\n").await;

    let tag = w.expect_id().await;
    w.upstream_send(format!("{tag} OK ID completed\r\n").as_bytes())
        .await;
    assert_eq!(w.client_line().await, "aA2 OK LOGIN completed\r\n");

    // Exactly one ID exchange happened; the session is transparent afterwards.
    w.client_send(b"aA3 NOOP\r\n").await;
    assert_eq!(w.upstream_line().await, "aA3 NOOP\r\n");
    w.upstream_send(b"aA3 OK NOOP completed\r\n").await;
    assert_eq!(w.client_line().await, "aA3 OK NOOP completed\r\n");
}

#[tokio::test]
async fn authenticate_sasl_then_binary_literals_pass_through() {
    let mut w = Wire::new();
    w.upstream_send(b"* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] ready\r\n")
        .await;
    assert_eq!(
        w.client_line().await,
        "* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] ready\r\n"
    );

    w.client_send(b"aA1 AUTHENTICATE PLAIN\r\n").await;
    assert_eq!(w.upstream_line().await, "aA1 AUTHENTICATE PLAIN\r\n");
    w.upstream_send(b"+ \r\n").await;
    assert_eq!(w.client_line().await, "+ \r\n");
    w.client_send(b"AGpvaG4AcGFzcw==\r\n").await;
    assert_eq!(w.upstream_line().await, "AGpvaG4AcGFzcw==\r\n");
    w.upstream_send(b"aA1 OK AUTHENTICATE completed\r\n").await;

    let tag = w.expect_id().await;
    w.upstream_send(format!("{tag} OK ID completed\r\n").as_bytes())
        .await;
    assert_eq!(w.client_line().await, "aA1 OK AUTHENTICATE completed\r\n");

    // A UID FETCH response with a large binary literal must cross untouched.
    w.client_send(b"aA2 UID FETCH 1 (BODY.PEEK[])\r\n").await;
    assert_eq!(w.upstream_line().await, "aA2 UID FETCH 1 (BODY.PEEK[])\r\n");
    let mut body: Vec<u8> = (0..48 * 1024).map(|i| (i * 37 % 256) as u8).collect();
    body.extend_from_slice(b"\r\n* 9 EXISTS\r\n");
    let mut response = format!("* 1 FETCH (UID 1 BODY[] {{{}}}\r\n", body.len()).into_bytes();
    response.extend_from_slice(&body);
    response.extend_from_slice(b")\r\naA2 OK FETCH completed\r\n");
    w.upstream_send(&response).await;
    let mut echoed = vec![0; response.len()];
    timeout(STEP, w.client_in.read_exact(&mut echoed))
        .await
        .expect("relay stalled forwarding the FETCH literal")
        .expect("client read failed");
    assert_eq!(echoed, response, "FETCH literal was not byte-for-byte");

    // A client APPEND literal must cross untouched as well.
    let mut blob: Vec<u8> = (0..777).map(|i| (i * 53 % 251) as u8).collect();
    blob.extend_from_slice(b"\r\n.\r\n");
    w.client_send(format!("aA3 APPEND INBOX {{{}}}\r\n", blob.len()).as_bytes())
        .await;
    assert_eq!(
        w.upstream_line().await,
        format!("aA3 APPEND INBOX {{{}}}\r\n", blob.len())
    );
    w.upstream_send(b"+ Ready for literal\r\n").await;
    assert_eq!(w.client_line().await, "+ Ready for literal\r\n");

    let mut sent = blob.clone();
    sent.extend_from_slice(b"\r\n");
    w.client_send(&sent).await;
    let mut received = vec![0; sent.len()];
    timeout(STEP, w.upstream_in.read_exact(&mut received))
        .await
        .expect("relay stalled forwarding the APPEND literal")
        .expect("server read failed");
    assert_eq!(received, sent, "APPEND literal was not byte-for-byte");

    w.upstream_send(b"aA3 OK APPEND completed\r\n").await;
    assert_eq!(w.client_line().await, "aA3 OK APPEND completed\r\n");
}
