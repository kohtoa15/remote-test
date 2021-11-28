# builder image for server binary
FROM rust:1.56 as builder

# Install protoc compiler
RUN apt update && \
    apt install -y protobuf-compiler

WORKDIR /usr/src/remote-test

COPY Cargo.toml .
COPY Cargo.lock .
COPY build.rs .
COPY src/ ./src
COPY proto/ ./proto

RUN rustup component add rustfmt
RUN cargo install --path . --bin server

# docker image for server binary
FROM debian:buster-slim

COPY --from=builder /usr/local/cargo/bin/server /usr/local/bin/remote-test-server

CMD ["remote-test-server"]
