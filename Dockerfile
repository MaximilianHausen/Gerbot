FROM rust:1.85-alpine AS builder
RUN apk add --no-cache build-base cmake
WORKDIR /source
COPY . .
RUN cargo build --release

FROM alpine:latest
RUN apk add --no-cache attr ca-certificates ffmpeg py3-brotli py3-mutagen py3-pycryptodomex py3-secretstorage py3-websockets
RUN wget https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -O /usr/bin/yt-dlp && chmod a+rx /usr/bin/yt-dlp
COPY --from=builder /source/target/release/gerbot /app/gerbot
WORKDIR /app
ENTRYPOINT ["/app/gerbot"]
