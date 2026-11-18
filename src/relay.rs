use std::{io, time::Duration};

use async_imap::imap_proto::{Response, Status, core::is_astring_char};
use tokio::{
    io::{self as aio, AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader},
    time::timeout,
};

mod wire;

const BUFFER: usize = 16 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(120);

pub async fn run(
    client: impl AsyncRead + AsyncWrite + Unpin,
    upstream: impl AsyncRead + AsyncWrite + Unpin,
    id_command: &str,
) -> io::Result<()> {
    let mut client = BufReader::with_capacity(BUFFER, client);
    let mut upstream = BufReader::with_capacity(BUFFER, upstream);
    if timeout(
        HANDSHAKE_TIMEOUT,
        handshake(&mut client, &mut upstream, id_command),
    )
    .await??
    {
        // Keep both readers so no pipelined commands or upstream read-ahead are lost.
        aio::copy_bidirectional_with_sizes(&mut client, &mut upstream, BUFFER, BUFFER).await?;
    }
    Ok(())
}

async fn handshake(
    client: &mut (impl AsyncBufRead + AsyncWrite + Unpin),
    upstream: &mut (impl AsyncBufRead + AsyncWrite + Unpin),
    id_command: &str,
) -> io::Result<bool> {
    let mut command = Vec::new();
    let mut response = Vec::new();
    wire::read_line(upstream, &mut response).await?;
    let (greeting, status) = wire::tokens(&response);
    if greeting != b"*" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid IMAP greeting",
        ));
    }
    if status.eq_ignore_ascii_case(b"PREAUTH") {
        wire::send(client, &response).await?;
        inject_id(client, upstream, &mut command, id_command).await?;
        return Ok(true);
    }
    wire::send(client, &response).await?;
    if !status.eq_ignore_ascii_case(b"OK") {
        return Ok(false);
    }

    loop {
        tokio::select! {
            ready = upstream.fill_buf() => {
                if ready?.is_empty() { return Ok(false); }
                wire::read_line(upstream, &mut response).await?;
                wire::response(upstream, client, &mut response, true).await?;
                continue;
            }
            ready = client.fill_buf() => {
                if ready?.is_empty() { return Ok(false); }
            }
        }
        wire::read_line(client, &mut command).await?;
        let (tag, verb) = wire::tokens(&command);
        if tag.is_empty() || !tag.iter().all(|&b| b != b'+' && is_astring_char(b)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid IMAP tag",
            ));
        }
        if verb.eq_ignore_ascii_case(b"STARTTLS") {
            let mut reply = tag.to_vec();
            reply.extend_from_slice(b" BAD STARTTLS is not supported by this listener\r\n");
            wire::send(client, &reply).await?;
            if wire::literal(&command)?.is_some_and(|(_, non_sync)| non_sync) {
                return Ok(false);
            }
            continue;
        }
        let is_sasl = verb.eq_ignore_ascii_case(b"AUTHENTICATE");
        // These mechanisms can switch framing as soon as authentication succeeds.
        let sasl_layer = is_sasl
            && command[..command.len() - 2]
                .split(|&b| b == b' ')
                .nth(2)
                .is_some_and(|mechanism| {
                    [
                        b"GSSAPI".as_slice(),
                        b"DIGEST-MD5",
                        b"KERBEROS_V4",
                        b"GSS-SPNEGO",
                    ]
                    .iter()
                    .any(|name| mechanism.eq_ignore_ascii_case(name))
                });
        if sasl_layer {
            inject_id(client, upstream, &mut response, id_command).await?;
        }
        let is_login = verb.eq_ignore_ascii_case(b"LOGIN");
        let logout = verb.eq_ignore_ascii_case(b"LOGOUT");
        let tag = tag.to_vec();
        // Do not send the following (possibly pipelined) command until the reply to this one.
        let mut reply = None;
        loop {
            wire::send(upstream, &command).await?;
            let Some((n, non_sync)) = wire::literal(&command)? else {
                break;
            };
            if !non_sync {
                let next = wait_reply(client, upstream, &mut response, &tag, false).await?;
                if !matches!(next, Reply::Continue) {
                    reply = Some(next);
                    break;
                }
                wire::send(client, &response).await?;
            }
            wire::copy_literal(client, Some(&mut *upstream), n).await?;
            wire::read_line(client, &mut command).await?;
        }
        let mut reply = match reply {
            Some(reply) => reply,
            None => wait_reply(client, upstream, &mut response, &tag, false).await?,
        };
        while matches!(reply, Reply::Continue) && is_sasl {
            wire::send(client, &response).await?;
            wire::read_line(client, &mut command).await?;
            wire::send(upstream, &command).await?;
            reply = wait_reply(client, upstream, &mut response, &tag, false).await?;
        }
        match reply {
            Reply::Continue => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected IMAP continuation",
                ));
            }
            Reply::Done(true) if is_login || is_sasl => {
                if !sasl_layer {
                    inject_id(client, upstream, &mut command, id_command).await?;
                }
                wire::send(client, &response).await?;
                return Ok(true);
            }
            Reply::Done(_) => wire::send(client, &response).await?,
        }
        if logout {
            return Ok(false);
        }
    }
}

enum Reply {
    Continue,
    Done(bool),
}

async fn wait_reply(
    client: &mut (impl AsyncWrite + Unpin),
    upstream: &mut (impl AsyncBufRead + Unpin),
    line: &mut Vec<u8>,
    expected: &[u8],
    internal_id: bool,
) -> io::Result<Reply> {
    loop {
        wire::read_line(upstream, line).await?;
        let (tag, status) = wire::tokens(line);
        if tag == expected {
            let status = match Response::parse(line) {
                Ok((
                    [],
                    Response::Done {
                        status: Status::Ok, ..
                    },
                )) => true,
                Ok((
                    [],
                    Response::Done {
                        status: Status::No | Status::Bad,
                        ..
                    },
                )) => false,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid IMAP completion",
                    ));
                }
            };
            return Ok(Reply::Done(status));
        }
        if tag == b"+" {
            return Ok(Reply::Continue);
        }
        let own_id = internal_id && tag == b"*" && status.eq_ignore_ascii_case(b"ID");
        wire::response(upstream, client, line, !own_id).await?;
    }
}

async fn inject_id(
    client: &mut (impl AsyncWrite + Unpin),
    upstream: &mut (impl AsyncBufRead + AsyncWrite + Unpin),
    response: &mut Vec<u8>,
    command: &str,
) -> io::Result<()> {
    // Pre-auth commands are serialized, so no client tag is outstanding here.
    const TAG: &str = "relay.id";
    wire::send(upstream, format!("{TAG} {command}\r\n").as_bytes()).await?;
    match wait_reply(client, upstream, response, TAG.as_bytes(), true).await? {
        Reply::Done(true) => Ok(()),
        Reply::Done(false) => {
            eprintln!("upstream rejected the relay's IMAP ID");
            Ok(())
        }
        Reply::Continue => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected IMAP ID continuation",
        )),
    }
}
