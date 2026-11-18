# imap-relay-netease

Rust/Tokio IMAP 中继。标准客户端或邮件归档工具无需实现 `ID` 扩展：中继在上游认证成功后、转发下一条命令前插入 [RFC 2971 `ID`](https://www.rfc-editor.org/rfc/rfc2971)。这是针对[网易邮箱 `Unsafe Login` 的官方接入要求](https://help.mail.163.com/faqDetail.do?code=d7a5dc8471cd0c0e8b4b8f4f8e49998b374173cfe9171305fa1ce630d7f67ac2eda07326646e6eb0)，不替代账户认证。设计参考 [163-imap-proxy](https://github.com/jizhi0v0/163-imap-proxy)。

## 启动

```sh
cargo build --release --locked
IMAP_LISTEN=127.0.0.1:1143 ./target/release/imap-relay-netease
```

本地测试时，将客户端连接到 `127.0.0.1:1143`，关闭客户端 TLS 和 STARTTLS。网易侧需开启 IMAP，客户端使用完整邮箱地址和网易生成的**客户端授权码**。不指定 `IMAP_LISTEN` 时监听 `127.0.0.1:143`；系统可能要求绑定低端口的权限。

| 环境变量 | 默认值 | 用途 |
| --- | --- | --- |
| `IMAP_LISTEN` | `127.0.0.1:143` | 明文监听地址，格式 `IP:PORT` |
| `IMAP_UPSTREAM_HOST` | `imap.163.com` | 上游主机名，用于连接和证书验证。126 可填 `imap.126.com`；企业或学校邮箱填写服务商给出的地址 |
| `IMAP_UPSTREAM_PORT` | `993` | 上游隐式 TLS 端口，不支持明文或 STARTTLS 上游 |
| `IMAP_ID_PROFILE` | `apple-mail` | `apple-mail`、`thunderbird`、`outlook` 或 `foxmail`；预设只发送产品名和厂商 |

一个实例连接一个上游。需要同时归档多个不同上游时，为每个上游启动一个实例。上游 TLS 使用系统信任库验证证书；私有 CA 可通过 `SSL_CERT_FILE` 指定。

## Traefik / Docker 私网

中继与 Traefik 同处私有 Docker 网络时，设置 `IMAP_LISTEN=0.0.0.0:143`，不要对外发布中继端口；只发布 Traefik 的 `993/tcp`。确保网络允许中继连接上游。以下为 Traefik 动态配置（静态配置另设 `imaps` 入口 `:993`，并为 `mail.example.com` 配置证书）：

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

客户端连接 `mail.example.com:993` 并启用隐式 TLS。Traefik 到中继这段仍是明文，包含授权码和邮件内容，需隔离在可信网络。中继不记录 IMAP 命令、凭据或邮件。

## 协议行为

- 认证阶段保留原始标签、SASL 交互和 IMAP 字面量。对已知可协商安全层的 SASL 机制，改在认证前发送 `ID`，避免在加密或完整性保护开启后插入明文命令。
- 隐藏注入的 `ID` 回复；若上游返回 `NO` 或 `BAD`，记录警告并继续，由上游决定后续命令是否可用。
- 认证后使用有背压的双向字节流，不解析或整封缓冲邮件。`LIST`、`UID FETCH`、`APPEND`、`IDLE` 和其他指令是否可用取决于上游。

本地协议测试：`cargo test --locked`。
