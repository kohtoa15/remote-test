# builder image for server binary
FROM rust:1.56 as builder

RUN rustup component add rustfmt

# Install protoc compiler
RUN apt update && \
    apt install -y protobuf-compiler

RUN cargo install --git https://github.com/kohtoa15/remote-test --bin server

# docker image for server binary
FROM debian:buster-slim

COPY --from=builder /usr/local/cargo/bin/server /usr/local/bin/remote-test-server

CMD ["remote-test-server"]
