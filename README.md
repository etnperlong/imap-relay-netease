<h1 align="center">imap-relay-netease</h1>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-Tokio-000000?style=flat-square&amp;logo=rust" alt="Rust / Tokio"></a>
  <a href="https://www.rfc-editor.org/rfc/rfc2971"><img src="https://img.shields.io/badge/IMAP-RFC_2971-2563eb?style=flat-square" alt="IMAP RFC 2971"></a>
  <a href="Dockerfile"><img src="https://img.shields.io/badge/Docker-Distroless-2496ed?style=flat-square&amp;logo=docker&amp;logoColor=white" alt="Docker / Distroless"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-15803d?style=flat-square" alt="MIT License"></a>
</p>

<p align="center">简体中文 · <a href="README_EN.md">English</a></p>

面向网易邮箱的 IMAP 中继。替客户端发送 [RFC 2971 `ID`](https://www.rfc-editor.org/rfc/rfc2971)，解决缺少 `ID` 导致的 [`Unsafe Login`](https://help.mail.163.com/faqDetail.do?code=d7a5dc8471cd0c0e8b4b8f4f8e49998b374173cfe9171305fa1ce630d7f67ac2eda07326646e6eb0)，无需修改邮件客户端或归档工具。

## 能力

- **自动发送 `ID`**：通常在上游认证后、转发下一条命令前发送，默认使用 Apple Mail 预设。上游拒绝 `ID` 时，中继记录警告并继续转发。
- **流式归档**：握手后按字节双向转发，不解析或整封缓冲邮件；`UID FETCH`、`APPEND` 和 `IDLE` 等命令由上游处理。
- **独立部署**：每个实例连接一个上游，可配置为 163、126 或自定义 IMAP 主机。上游始终使用带证书校验的隐式 TLS。

## 快速开始

```sh
IMAP_LISTEN=127.0.0.1:1143 cargo run --release --locked
```

先在网易网页端开启 IMAP 并生成**客户端授权码**。将客户端连到 `127.0.0.1:1143`，关闭 TLS 和 STARTTLS，以完整邮箱地址和授权码登录。

## 配置

| 环境变量 | 默认值 | 用途 |
| --- | --- | --- |
| `IMAP_LISTEN` | `127.0.0.1:143`（容器内 `0.0.0.0:143`） | 明文监听地址，格式为 `IP:PORT` |
| `IMAP_UPSTREAM_HOST` | `imap.163.com` | 上游主机名，用于连接和证书校验；126 邮箱可填 `imap.126.com` |
| `IMAP_UPSTREAM_PORT` | `993` | 上游隐式 TLS 端口 |
| `IMAP_ID_PROFILE` | `apple-mail` | 也可设为 `thunderbird`、`outlook` 或 `foxmail` |

私有上游 CA 可通过 `SSL_CERT_FILE` 指定。中继不记录 IMAP 命令、凭据或邮件。

## Docker 与 Traefik

[Dockerfile](Dockerfile) 使用 musl 静态构建和 Distroless Debian 13 非 root 运行镜像。假设 Traefik 已加入 `traefik` 网络，从项目根目录运行：

```sh
docker build --pull -t imap-relay-netease .
docker run -d --name imap-relay --network traefik --restart unless-stopped \
  --init --cap-drop=ALL \
  --sysctl net.ipv4.ip_unprivileged_port_start=143 \
  imap-relay-netease
```

`--sysctl` 允许非 root 在容器自己的网络命名空间绑定 `143`，不要搭配 host 网络。若平台不允许设置它，去掉 `--sysctl`，用 `-e IMAP_LISTEN=0.0.0.0:1143`，并同步修改后端端口。

> [!WARNING]
> Traefik 到中继仍是明文，包含授权码和邮件内容。请将中继放在可信私网，不发布容器端口；只对外开放 Traefik 的 `993/tcp`。

先在 Traefik 静态配置中定义 `imaps` 入口 `:993` 并配置域名证书，再添加 TCP 路由：

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

客户端连接 `mail.example.com:993`，启用隐式 TLS。中继不提供 STARTTLS。

## 发布与镜像

推送与 `Cargo.toml` 版本一致的 `vX.Y.Z` 标签会触发 GitHub Release，为 Linux（musl，完全静态）、Windows（静态 C 运行库，仅依赖系统 DLL）和 macOS（系统库与框架，非完全静态）各提供 amd64 与 arm64 二进制。Linux 和 macOS 使用 `.tar.gz`，Windows 使用 `.zip`，每个压缩包内含可执行文件和 `LICENSE`，并附 `SHA256SUMS`。

同一标签会构建 `ghcr.io/<owner>/imap-relay-netease` 的 `linux/amd64` 和 `linux/arm64` 镜像。稳定版使用 `vX.Y.Z` 和 `latest`（最近推送的稳定镜像），预发布只使用精确标签（如 `vX.Y.Z-rc.1`）。将示例中的 `your-github-owner` 改为仓库所有者的小写名称：

```sh
docker pull ghcr.io/your-github-owner/imap-relay-netease:latest
```

发布标签需带 `v` 前缀且不含 `+` 构建元数据。可在 Actions 手动运行 `Release` 来验证构建，不会推送镜像或创建 Release。首次发布 GHCR 镜像后，如需匿名拉取，请将包设为公开。

## 验证

`cargo test --locked` 覆盖认证重试、SASL、`ID` 注入顺序和二进制邮件转发。

## 参考项目

设计参考 [jizhi0v0/163-imap-proxy](https://github.com/jizhi0v0/163-imap-proxy)。

---

本项目采用 [MIT 许可证](LICENSE)。
