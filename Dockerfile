FROM docker.io/library/rust:1.96-alpine3.24 AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN target="$(rustc -vV | awk '/^host:/ {print $2}')" \
    && RUSTFLAGS="-C target-feature=+crt-static -C strip=symbols" \
       cargo build --locked --release --target "$target" \
    && cp "target/$target/release/imap-relay-netease" /imap-relay-netease

FROM gcr.io/distroless/static-debian13:nonroot
COPY --from=build /imap-relay-netease /imap-relay-netease
ENV IMAP_LISTEN=0.0.0.0:143
EXPOSE 143/tcp
ENTRYPOINT ["/imap-relay-netease"]
