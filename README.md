# git2-rs

[![Build Status](https://travis-ci.org/alexcrichton/git2-rs.svg?branch=master)](https://travis-ci.org/alexcrichton/git2-rs)

[Documentation](http://alexcrichton.com/git2-rs/git2/index.html)

libgit2 bindings for Rust

```toml
[dependencies]
git2 = "0.1"
```

## Building git2-rs

First, you'll need to install _CMake_ and _pkg-config_ if you don't already
have libgit2 installed elsewhere on the system. Afterwards, just run:

```sh
$ git clone --recursive https://github.com/alexcrichton/git2-rs
$ cd git2-rs
$ cargo build
```

# License

`git2-rs` is primarily distributed under the terms of both the MIT license and
the Apache License (Version 2.0), with portions covered by various BSD-like
licenses.

See LICENSE-APACHE, and LICENSE-MIT for details.
