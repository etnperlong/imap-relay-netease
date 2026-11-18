<h1 align="center">imap-relay-netease</h1>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-Tokio-000000?style=flat-square&amp;logo=rust" alt="Rust / Tokio"></a>
  <a href="https://www.rfc-editor.org/rfc/rfc2971"><img src="https://img.shields.io/badge/IMAP-RFC_2971-2563eb?style=flat-square" alt="IMAP RFC 2971"></a>
  <a href="Dockerfile"><img src="https://img.shields.io/badge/Docker-Distroless-2496ed?style=flat-square&amp;logo=docker&amp;logoColor=white" alt="Docker / Distroless"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-15803d?style=flat-square" alt="MIT License"></a>
</p>

<p align="center"><a href="README.md">简体中文</a> · English</p>

An IMAP relay for NetEase Mail. It sends the [RFC 2971 `ID` command](https://www.rfc-editor.org/rfc/rfc2971) on behalf of your client, preventing the [`Unsafe Login` error](https://help.mail.163.com/faqDetail.do?code=d7a5dc8471cd0c0e8b4b8f4f8e49998b374173cfe9171305fa1ce630d7f67ac2eda07326646e6eb0) caused by a missing `ID`. No changes to your mail client or archiving tool are required.

## Features

- **Automatic IMAP identification:** The relay normally sends `ID` after upstream authentication and before forwarding the next command. It uses the Apple Mail preset by default. If the server rejects `ID`, the relay logs a warning and continues forwarding.
- **Streaming for mail archives:** After the handshake, the relay forwards bytes in both directions without parsing or buffering entire messages. The upstream server handles commands such as `UID FETCH`, `APPEND`, and `IDLE`.
- **Configurable upstream:** Each instance connects to one IMAP server, such as 163 Mail, 126 Mail, or a custom host. The upstream connection always uses implicit TLS with certificate verification.

## Quick start

```sh
IMAP_LISTEN=127.0.0.1:1143 cargo run --release --locked
```

Enable IMAP in NetEase webmail and generate a **client authorization code**. Connect your client to `127.0.0.1:1143` with TLS and STARTTLS disabled. Sign in with your full email address and the authorization code, not your webmail password.

## Configuration

| Environment variable | Default | Purpose |
| --- | --- | --- |
| `IMAP_LISTEN` | `127.0.0.1:143` (`0.0.0.0:143` in the container) | Plaintext listener, in `IP:PORT` format |
| `IMAP_UPSTREAM_HOST` | `imap.163.com` | Server hostname used for the connection and certificate verification. Use `imap.126.com` for 126 Mail. |
| `IMAP_UPSTREAM_PORT` | `993` | Upstream implicit TLS port |
| `IMAP_ID_PROFILE` | `apple-mail` | Also accepts `thunderbird`, `outlook`, and `foxmail` |

Use `SSL_CERT_FILE` to specify a private CA for the upstream connection. The relay does not log IMAP commands, credentials, or message contents.

## Docker and Traefik

The [Dockerfile](Dockerfile) builds a static musl binary and runs it as a non-root user in Distroless Debian 13. If Traefik is already on the `traefik` network, run these commands from the project root:

```sh
docker build --pull -t imap-relay-netease .
docker run -d --name imap-relay --network traefik --restart unless-stopped \
  --init --cap-drop=ALL \
  --sysctl net.ipv4.ip_unprivileged_port_start=143 \
  imap-relay-netease
```

The `--sysctl` option lets the non-root process bind port `143` in the container's network namespace. Do not use it with host networking. If your platform does not allow this option, omit it, add `-e IMAP_LISTEN=0.0.0.0:1143`, and change the backend port below to `1143`.

> [!WARNING]
> Traffic from Traefik to the relay is plaintext and carries authorization codes and email. Keep the relay on a trusted private network and do not publish its port. Expose only Traefik's `993/tcp` port.

Define an `imaps` entry point on `:993` in Traefik's static configuration, configure a certificate for your domain, and add this TCP route:

```yaml
tcp:
  routers:
    imaps:
      entryPoints: [imaps]
      rule: "HostSNI(`mail.example.com`)"
      service: imap-relay
      tls:
        passthrough: false
  services:
    imap-relay:
      loadBalancer:
        servers:
          - address: "imap-relay:143"
```

Connect your mail client to `mail.example.com:993` with implicit TLS enabled. The relay does not offer STARTTLS.

## Verification

Run `cargo test --locked` to test authentication retries, SASL, `ID` ordering, and binary message forwarding.

## Reference project

The relay design was inspired by [jizhi0v0/163-imap-proxy](https://github.com/jizhi0v0/163-imap-proxy).

---

This project is available under the [MIT License](LICENSE).
