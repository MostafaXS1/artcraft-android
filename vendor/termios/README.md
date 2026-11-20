# Termios Rust Bindings

The `termios` crate provides safe bindings for the Rust programming language to the [terminal I/O
interface](http://pubs.opengroup.org/onlinepubs/009695399/basedefs/termios.h.html) implemented by
Unix operating systems. The safe bindings are a small wrapper around the raw C functions, which
converts integer return values to `std::io::Result` to indicate success or failure.

* [Documentation](http://dcuddeback.github.io/termios-rs/termios/)

## Dependencies
In order to use the `termios` crate, you must have a native `libc` library that implements the
termios API. This should be available on any Unix operating system. It has been confirmed to work on
the following platforms:

* Linux (x86_64, armv6l)
* OS X (x86_64)
* FreeBSD (amd64)
* OpenBSD (amd64)

## Usage
Add `termios` as a dependency in `Cargo.toml`:

```toml
[dependencies]
termios = "0.2"
```

Import the `termios` crate and any symbols needed from `termios`. You may also need
`std::os::unix::io::RawFd` for file descriptors and `std::io::Result` to propagate errors.

```rust
extern crate termios;

use std::io;
use std::os::unix::io::RawFd;

use termios::*;

fn setup_fd(fd: RawFd) -> io::Result<()> {
  let mut termios = try!(Termios::from_fd(fd));

  termios.c_iflag = IGNPAR | IGNBRK;
  termios.c_oflag = 0;
  termios.c_cflag = CS8 | CREAD | CLOCAL;
  termios.c_lflag = 0;

  try!(cfsetspeed(&mut termios, B9600));
  try!(tcsetattr(fd, TCSANOW, &termios));
  try!(tcflush(fd, TCIOFLUSH));

  Ok(())
}
```

## Contributors
* [dcuddeback](https://github.com/dcuddeback/)
* [conradkleinespel](https://github.com/conradkleinespel)
* [myfreeweb](https://github.com/myfreeweb)
* [qbit](https://github.com/qbit)

## License
Copyright © 2015 David Cuddeback

Distributed under the [MIT License](LICENSE).
