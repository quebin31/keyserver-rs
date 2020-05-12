FROM rust:latest as cargo-build

# Install dependencies
RUN apt-get update && apt-get install -y \
    clang-3.9 \
    libssl-dev \
    libzmq3-dev

WORKDIR /usr/src/keyserver

# Dummy compile
COPY Cargo.toml Cargo.lock ./
RUN mkdir src/
RUN echo "fn main() {println!(\"failed to replace dummy build\")}" > src/main.rs
RUN cargo build --release
RUN rm -f target/release/deps/keyserver*

# Compile
COPY . .
RUN cargo build --release

FROM ubuntu:latest

RUN apt-get update && apt-get install -y libssl-dev libzmq3-dev

COPY --from=cargo-build /usr/src/keyserver/target/release/keyserver /usr/local/bin/keyserver

ENTRYPOINT ["keyserver"]