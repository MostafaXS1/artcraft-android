//! Unsafe FFI bindings.

extern crate libc;

use libc::{c_int,pid_t};

#[link(name = "c")]
extern "C" {
    pub fn tcgetattr(fd: c_int, termios_p: *mut ::os::target::termios) -> c_int;
    pub fn tcsetattr(fd: c_int, optional_actions: c_int, termios_p: *const ::os::target::termios) -> c_int;
    pub fn tcsendbreak(fd: c_int, duration: c_int) -> c_int;
    pub fn tcdrain(fd: c_int) -> c_int;
    pub fn tcflush(fd: c_int, queue_selector: c_int) -> c_int;
    pub fn tcflow(fd: c_int, action: c_int) -> c_int;
    pub fn cfmakeraw(termios_p: *mut ::os::target::termios);
    pub fn cfgetispeed(termios_p: *const ::os::target::termios) -> ::os::target::speed_t;
    pub fn cfgetospeed(termios_p: *const ::os::target::termios) -> ::os::target::speed_t;
    pub fn cfsetispeed(termios_p: *mut ::os::target::termios, speed: ::os::target::speed_t) -> c_int;
    pub fn cfsetospeed(termios_p: *mut ::os::target::termios, speed: ::os::target::speed_t) -> c_int;
    pub fn cfsetspeed(termios_p: *mut ::os::target::termios, speed: ::os::target::speed_t) -> c_int;
    pub fn tcgetsid(fd: c_int) -> pid_t;
}
