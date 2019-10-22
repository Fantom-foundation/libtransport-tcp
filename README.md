libtransport-tcp
===========
![Rust: nightly](https://img.shields.io/badge/Rust-nightly-blue.svg) ![License: MIT](https://img.shields.io/badge/License-MIT-green.svg) [![Build Status](https://travis-ci.org/Fantom-foundation/evm-rs.svg?branch=master)](https://travis-ci.org/Fantom-foundation/evm-rs)

libtransport-tcp in Rust.

## RFCs

https://github.com/Fantom-foundation/fantom-rfcs

# Developer guide

Install the latest version of [Rust](https://www.rust-lang.org). We tend to use nightly versions. [CLI tool for installing Rust](https://rustup.rs).

We use [rust-clippy](https://github.com/rust-lang-nursery/rust-clippy) linters to improve code quality.

There are plenty of [IDEs](https://areweideyet.com) and other [Rust development tools to consider](https://github.com/rust-unofficial/awesome-rust#development-tools).

This crate provides a TCP-based implementation of the libtransport crate. Other data structures such as the Peer, PeerId,
PeerList and Data must be defined by the developer for their individual use case. A simple example of a method which can
use this trait is the common_test method in libtransport:

```rust
// Create data type to be sent over TCP stream
struct data (pub u8);

// Create a set of peers with associated addresses and sockets
let a: Vec<String> = vec![
    String::from("127.0.0.1:9000"),
    String::from("127.0.0.1:9001"),
    String::from("127.0.0.1:9002"),
];

// Call the method, using the transport type, with the peers inputted.
common_test::<TCPtransport<data>>(a);
```

When implementing this yourself, you would obviously need to implement more features and traits. This is just a general
idea of how it could work. For a more specific example, refer to the common_test method in libtransport/generic_tests
(https://github.com/Fantom-foundation/libtransport/blob/master/src/generic_test.rs).

### Step-by-step guide
```bash
# Install Rust (nightly)
$ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain nightly
# Install cargo-make (cross-platform feature-rich reimplementation of Make)
$ cargo install --force cargo-make
# Install rustfmt (Rust formatter)
$ rustup component add rustfmt
# Clone this repo
$ git clone https://github.com/Fantom-foundation/libtransport-tcp && cd libtransport-tcp
# Run tests
$ cargo test
# Format, build and test
$ cargo make
```
