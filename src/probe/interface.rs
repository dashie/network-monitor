//! Platform interface binding and validation.
//!
//! This is the only module that knows how each operating system binds a socket to a
//! network interface. Everything above it works on interface names.

use std::ffi::CString;

/// Verify that the configured interface can be used, before the first probe (BR-5).
#[cfg(unix)]
pub fn validate(name: &str) -> Result<(), String> {
    let interface =
        CString::new(name).map_err(|_| format!("interface '{name}' contains a NUL byte"))?;
    let index = unsafe { libc::if_nametoindex(interface.as_ptr()) };
    if index == 0 {
        Err(format!("interface '{name}' does not exist on this host"))
    } else {
        Ok(())
    }
}

/// Verify that the configured interface can be used, before the first probe (BR-5).
#[cfg(windows)]
pub fn validate(name: &str) -> Result<(), String> {
    match name.parse::<u32>() {
        Ok(0) => Err(format!(
            "interface '{name}' must be a numeric interface index on Windows"
        )),
        Ok(_) => Ok(()),
        Err(_) => Err(format!(
            "interface '{name}' must be a numeric interface index on Windows; \
             name resolution is not implemented yet"
        )),
    }
}

/// Bind a socket to an interface.
#[cfg(unix)]
pub fn bind(fd: std::os::fd::RawFd, name: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        // SO_BINDTODEVICE, Linux only, needs CAP_NET_RAW.
        const SO_BINDTODEVICE: libc::c_int = 25;
        let interface =
            CString::new(name).map_err(|_| format!("interface '{name}' contains a NUL byte"))?;
        let length = (interface.as_bytes().len() + 1) as libc::socklen_t;
        let result = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                SO_BINDTODEVICE,
                interface.as_ptr() as *const libc::c_void,
                length,
            )
        };
        if result != 0 {
            return Err(format!(
                "cannot bind the socket to interface '{name}': {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        // IP_BOUND_IF, BSD family.
        const IP_BOUND_IF: libc::c_int = 25;
        let interface =
            CString::new(name).map_err(|_| format!("interface '{name}' contains a NUL byte"))?;
        let index = unsafe { libc::if_nametoindex(interface.as_ptr()) };
        if index == 0 {
            return Err(format!("interface '{name}' does not exist on this host"));
        }
        let value = index as libc::c_uint;
        let result = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_IP,
                IP_BOUND_IF,
                &value as *const libc::c_uint as *const libc::c_void,
                std::mem::size_of::<libc::c_uint>() as libc::socklen_t,
            )
        };
        if result != 0 {
            return Err(format!(
                "cannot bind the socket to interface '{name}': {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    {
        let _ = (fd, name);
        Err("interface binding is not implemented for this operating system".to_string())
    }
}

/// Bind a socket to an interface. Windows identifies interfaces by numeric index.
#[cfg(windows)]
pub fn bind(socket: usize, name: &str) -> Result<(), String> {
    const IPPROTO_IP: i32 = 0;
    const IP_UNICAST_IF: i32 = 31;

    #[link(name = "ws2_32")]
    extern "system" {
        fn setsockopt(socket: usize, level: i32, name: i32, value: *const u8, length: i32) -> i32;
        fn WSAGetLastError() -> i32;
    }

    let index: u32 = name
        .parse()
        .map_err(|_| format!("interface '{name}' must be a numeric interface index on Windows"))?;
    let value = index.to_be_bytes();
    let result = unsafe { setsockopt(socket, IPPROTO_IP, IP_UNICAST_IF, value.as_ptr(), 4) };
    if result != 0 {
        let code = unsafe { WSAGetLastError() };
        return Err(format!(
            "cannot bind the socket to interface index {index} (WSA error {code})"
        ));
    }
    Ok(())
}
