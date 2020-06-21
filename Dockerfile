FROM rust:latest as cargo-build

# Install dependencies
RUN apt-get update && apt-get install -y \
    clang-3.9 \
    libssl-dev \
    libzmq3-dev

WORKDIR /app

# Dummy compile
COPY Cargo.toml Cargo.lock ./
RUN mkdir src/
RUN echo "fn main() {println!(\"failed to replace dummy build\")}" > src/main.rs
RUN cargo build --release --all-features
RUN rm -f target/release/deps/keyserver*

# Compile
COPY . .
RUN cargo build --release --all-features

FROM ubuntu:latest

RUN apt-get update && apt-get install -y libssl-dev libzmq3-dev

COPY --from=cargo-build /app/target/release/keyserver /usr/local/bin/keyserver
COPY --from=cargo-build /app/static /static

ENTRYPOINT ["keyserver"]
