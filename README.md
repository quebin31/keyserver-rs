<h1 align="center">
  Cash:web Keyserver
</h1>

<p align="center">
  A Bitcoin Cash public key and metadata registry.
</p>

<p align="center">
  <a href="https://github.com/hlb8122/line-reader/actions">
    <img alt="Build Status" src="https://github.com/hlb8122/line-reader/workflows/CI/badge.svg">
  </a>

  <a href="LICENSE">
    <img alt="License" src="https://img.shields.io/badge/license-MIT-blue.svg">
  </a>
</p>

This repository hosts a reference implementation of the Cash:web Keyserver protocol. The goal is to provide a distributed, simple-to-use and cryptographically verifiable way to look up public keys, and other metadata, from their hashes. The hashes are commonly available within Bitcoin Cash Addresses such as *bitcoincash:pqkh9ahfj069qv8l6eysyufazpe4fdjq3u4hna323j*.

## Why not existing systems?

Traditional keyservers are subject to certificate spamming attacks. By being a first-class citizen in the cryptocurrency ecosystem, we are able to charge for key updates. This prevents an explosion of advertised certificates. Other systems like OpenAlias, require that you trust the service provider is providing the correct addresses, while this keyserver cannot forge such updates. At most, a malicious keyserver can censor a particular key, in which case other keyservers in the network will provide it.

## Running a Server

### Setting up Bitcoin

Bitcoin must be running with [RPC](https://bitcoin.org/en/developer-reference#remote-procedure-calls-rpcs) enabled.

### Enabling Prometheus (optional)

One can optionally enable a [Prometheus](https://prometheus.io/) exporter, by compiling using the `--feature monitoring` feature flag.

### Build

Install [Rust](https://www.rust-lang.org/tools/install) then

```bash
sudo apt install -y clang pkg-config libssl-dev
cargo build --release
```

The executable will be located at `./target/release/keyserver`.

### Configuration

Settings may be given by `JSON`, `TOML`, `YAML`, `HJSON` and `INI` files and, by default, are located at `~/.keyserver/config.*`. 

The `--config` argument will override the default location for the configuration file. Additional command-line arguments, given in the example below, will override the values given in the configuration file. Executing `keyserver --help` will give an exhaustive list of options available.

All data sizes are given in bytes, prices in satoshis, and durations in milliseconds.

In TOML format, the default values are as follows:

```toml
# The bind address for the server
# --bind
bind = "127.0.0.1:8080"

# Bind address for the prometheus exporter
# --bind-prom
bind_prom = "127.0.0.1:9095"

# Bitcoin network
# --network
# NOTE: Allowed values are "mainnet", "testnet", and "regtest".
network = "regtest"

# Database path
# --db-path
db_path = "~/.keyserver/db"

[bitcoin_rpc]
# Bitcoin RPC address
# --rpc-addr
address = "http://127.0.0.1:18443"

# Bitcoin RPC username
# --rpc-username
username = "user"

# Bitcoin RPC password
# --rpc-password
password = "password"

[limits]
# Maximum metadata size (5 Kb)
metadata_size = 5_120

# Maximum payment size (3 Kb)
payment_size = 3_072

[payments]
# The payment timeout
timeout = 60_000

# The price of a POP token
token_fee = 100_000

# BIP70 payment memo
memo = "Thanks for your custom!"

# HMAC secret, given in hexidecimal
# --hmac-secret
# NOTE: This will not be given a default value in release compilation due to security considerations.
hmac_secret = "1234"

[peering]
# Whether peering should be enabled
enabled = true

# Maximum number of peers
max_peers = 128

```

### Running

```bash
./target/release/keyserver [OPTIONS]
```

Alternatively, copy `./static/` folder and `keyserver` to a directory and run `keyserver` from there.
