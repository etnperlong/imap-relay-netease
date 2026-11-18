use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_LINE: usize = 64 * 1024;

pub async fn read_line(
    reader: &mut (impl AsyncBufRead + Unpin),
    line: &mut Vec<u8>,
) -> io::Result<()> {
    line.clear();
    let read = reader
        .take((MAX_LINE + 1) as u64)
        .read_until(b'\n', line)
        .await?;
    if read > MAX_LINE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IMAP line too long",
        ));
    }
    if !line.ends_with(b"\r\n") {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "incomplete IMAP line",
        ));
    }
    Ok(())
}

pub fn tokens(line: &[u8]) -> (&[u8], &[u8]) {
    let mut parts = line[..line.len() - 2].splitn(3, |&b| b == b' ');
    (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    )
}

pub fn literal(line: &[u8]) -> io::Result<Option<(u64, bool)>> {
    let Some(end) = line.strip_suffix(b"}\r\n") else {
        return Ok(None);
    };
    let Some(open) = end.iter().rposition(|&b| b == b'{') else {
        return Ok(None);
    };
    let prefix = end[..open].strip_suffix(b"~").unwrap_or(&end[..open]);
    if !prefix.is_empty() && !prefix.ends_with(b" ") && !prefix.ends_with(b"(") {
        return Ok(None);
    }
    let number = &end[open + 1..];
    let digits = number.strip_suffix(b"+").unwrap_or(number);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Ok(None);
    }
    let n = str::from_utf8(digits)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "IMAP literal length overflow")
        })?;
    Ok(Some((n, number != digits)))
}

pub async fn copy_literal(
    source: &mut (impl AsyncBufRead + Unpin),
    mut dest: Option<&mut (impl AsyncWrite + Unpin)>,
    mut remaining: u64,
) -> io::Result<()> {
    while remaining > 0 {
        let buf = source.fill_buf().await?;
        if buf.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete IMAP literal",
            ));
        }
        let n = remaining.min(buf.len() as u64) as usize;
        if let Some(writer) = &mut dest {
            writer.write_all(&buf[..n]).await?;
        }
        source.consume(n);
        remaining -= n as u64;
    }
    Ok(())
}

pub async fn response(
    source: &mut (impl AsyncBufRead + Unpin),
    dest: &mut (impl AsyncWrite + Unpin),
    line: &mut Vec<u8>,
    send: bool,
) -> io::Result<()> {
    let (tag, status) = tokens(line);
    // Response text in status and continuation lines can end with arbitrary braces.
    let data = tag == b"*"
        && ![b"OK".as_slice(), b"NO", b"BAD", b"BYE", b"PREAUTH"]
            .iter()
            .any(|s| status.eq_ignore_ascii_case(s));
    loop {
        if send {
            dest.write_all(line).await?;
        }
        if !data {
            break;
        }
        let Some((n, _)) = literal(line)? else { break };
        copy_literal(source, send.then_some(&mut *dest), n).await?;
        read_line(source, line).await?;
    }
    if send {
        dest.flush().await?;
    }
    Ok(())
}

pub async fn send(writer: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) -> io::Result<()> {
    writer.write_all(bytes).await?;
    writer.flush().await
}
