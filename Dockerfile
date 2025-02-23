FROM rust:1.85-alpine AS builder
RUN apk add --no-cache build-base cmake
WORKDIR /source
COPY . .
RUN cargo build --release

FROM alpine:latest
RUN apk add --no-cache yt-dlp-core
COPY --from=builder /source/target/release/gerbot /app/gerbot
WORKDIR /app
ENTRYPOINT ["/app/gerbot"]
