//! Linux SocketCAN backend with explicit non-Linux shims.

use cantools_core::{CanFrame, CaptureEvent, FrameSink, FrameSource};
use thiserror::Error;

/// SocketCAN backend errors.
#[derive(Debug, Error)]
pub enum Error {
    /// I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The current platform does not support SocketCAN.
    #[error("SocketCAN backends are only available on Linux")]
    UnsupportedPlatform,
    /// The named interface could not be resolved.
    #[error("unknown CAN interface {0}")]
    UnknownInterface(String),
    /// Core frame validation failed.
    #[error(transparent)]
    Core(#[from] cantools_core::CoreError),
}

/// Result type for backend operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Minimal interface description surfaced by the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceInfo {
    /// Interface name.
    pub name: String,
    /// Numeric ifindex.
    pub index: u32,
}

/// List interfaces visible to the backend.
pub fn interfaces() -> Result<Vec<InterfaceInfo>> {
    platform::interfaces()
}

/// Linux raw CAN socket wrapper.
pub struct RawSocket {
    inner: platform::RawSocketImpl,
}

impl RawSocket {
    /// Open and bind a raw CAN socket on the named interface.
    pub fn open(interface: &str) -> Result<Self> {
        Ok(Self {
            inner: platform::RawSocketImpl::open(interface)?,
        })
    }

    /// Set nonblocking mode on the socket.
    pub fn set_nonblocking(&self, enabled: bool) -> Result<()> {
        self.inner.set_nonblocking(enabled)
    }

    /// Receive the next frame envelope from the socket.
    pub fn recv_event(&self) -> Result<Option<CaptureEvent>> {
        self.inner.recv_event()
    }
}

impl FrameSink for RawSocket {
    type Error = Error;

    fn send(&mut self, frame: &CanFrame) -> std::result::Result<(), Self::Error> {
        self.inner.send(frame)
    }
}

impl FrameSource for RawSocket {
    type Error = Error;

    fn recv(&mut self) -> std::result::Result<Option<CanFrame>, Self::Error> {
        self.inner.recv()
    }
}

/// Linux ISO-TP socket wrapper.
pub struct IsotpSocket {
    inner: platform::IsotpSocketImpl,
}

impl IsotpSocket {
    /// Open and bind an ISO-TP socket.
    pub fn open(interface: &str, tx_id: u32, rx_id: u32) -> Result<Self> {
        Ok(Self {
            inner: platform::IsotpSocketImpl::open(interface, tx_id, rx_id)?,
        })
    }

    /// Send a protocol payload.
    pub fn send(&self, payload: &[u8]) -> Result<()> {
        self.inner.send(payload)
    }

    /// Receive a protocol payload.
    pub fn recv(&self, buffer: &mut [u8]) -> Result<usize> {
        self.inner.recv(buffer)
    }
}

/// Linux J1939 socket wrapper.
pub struct J1939Socket {
    inner: platform::J1939SocketImpl,
}

impl J1939Socket {
    /// Open and bind a J1939 socket.
    pub fn open(interface: &str, name: u64, pgn: u32, address: u8) -> Result<Self> {
        Ok(Self {
            inner: platform::J1939SocketImpl::open(interface, name, pgn, address)?,
        })
    }

    /// Send a J1939 payload.
    pub fn send(&self, payload: &[u8]) -> Result<()> {
        self.inner.send(payload)
    }

    /// Receive a J1939 payload.
    pub fn recv(&self, buffer: &mut [u8]) -> Result<usize> {
        self.inner.recv(buffer)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::{
        ffi::CString,
        io,
        mem::{size_of, zeroed},
        os::fd::RawFd,
        time::SystemTime,
    };

    use cantools_core::{
        CanFrame, CanId, CaptureEvent, Direction, FdFlags, FrameClass, InterfaceRef, Timestamp,
    };
    use libc::{c_int, c_void};

    use super::{Error, InterfaceInfo, Result};

    const SOL_CAN_RAW: c_int = 101;
    const CAN_RAW: c_int = 1;
    const CAN_RAW_FD_FRAMES: c_int = 5;
    const CAN_ISOTP: c_int = 6;
    const CAN_J1939: c_int = 7;
    const CAN_EFF_FLAG: u32 = 0x8000_0000;
    const CAN_RTR_FLAG: u32 = 0x4000_0000;
    const CAN_ERR_FLAG: u32 = 0x2000_0000;
    const CAN_EFF_MASK: u32 = 0x1fff_ffff;
    const CANFD_BRS: u8 = 0x01;
    const CANFD_ESI: u8 = 0x02;

    #[repr(C)]
    struct CanFrameNative {
        can_id: u32,
        can_dlc: u8,
        len8_dlc: u8,
        pad: u8,
        res0: u8,
        data: [u8; 8],
    }

    #[repr(C)]
    struct CanFdFrameNative {
        can_id: u32,
        len: u8,
        flags: u8,
        res0: u8,
        res1: u8,
        data: [u8; 64],
    }

    #[repr(C)]
    struct SockAddrCan {
        can_family: libc::sa_family_t,
        can_ifindex: c_int,
        can_addr: CanAddr,
    }

    #[repr(C)]
    union CanAddr {
        tp: CanTpAddr,
        j1939: CanJ1939Addr,
        align: [u8; 8],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CanTpAddr {
        rx_id: u32,
        tx_id: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CanJ1939Addr {
        name: u64,
        pgn: u32,
        addr: u8,
    }

    fn errno() -> io::Error {
        io::Error::last_os_error()
    }

    fn can_id_from_frame(frame: &CanFrame) -> u32 {
        let mut raw = frame.id.raw();
        if frame.id.is_extended() {
            raw |= CAN_EFF_FLAG;
        }
        match frame.class {
            FrameClass::Remote => raw |= CAN_RTR_FLAG,
            FrameClass::Error => raw |= CAN_ERR_FLAG,
            FrameClass::Data => {}
        }
        raw
    }

    fn interface_index(interface: &str) -> Result<u32> {
        let name =
            CString::new(interface).map_err(|_| Error::UnknownInterface(interface.to_string()))?;
        // SAFETY: `name` is a valid NUL-terminated interface string.
        let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
        if index == 0 {
            Err(Error::UnknownInterface(interface.to_string()))
        } else {
            Ok(index)
        }
    }

    fn set_fd_frames(fd: RawFd) -> Result<()> {
        let enable: c_int = 1;
        // SAFETY: `fd` is an owned socket, the option buffer points to a valid integer, and the size matches.
        let result = unsafe {
            libc::setsockopt(
                fd,
                SOL_CAN_RAW,
                CAN_RAW_FD_FRAMES,
                &enable as *const _ as *const c_void,
                size_of::<c_int>() as libc::socklen_t,
            )
        };
        if result == -1 {
            Err(Error::Io(errno()))
        } else {
            Ok(())
        }
    }

    fn bind_raw_socket(fd: RawFd, interface: &str) -> Result<()> {
        let ifindex = interface_index(interface)? as c_int;
        let addr = SockAddrCan {
            can_family: libc::AF_CAN as libc::sa_family_t,
            can_ifindex: ifindex,
            can_addr: CanAddr { align: [0; 8] },
        };
        // SAFETY: `addr` is a valid sockaddr_can and the socket is open.
        let result = unsafe {
            libc::bind(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                size_of::<SockAddrCan>() as libc::socklen_t,
            )
        };
        if result == -1 {
            Err(Error::Io(errno()))
        } else {
            Ok(())
        }
    }

    fn set_nonblocking(fd: RawFd, enabled: bool) -> Result<()> {
        // SAFETY: `fcntl` is called on a valid fd with a pure query command.
        let current = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if current == -1 {
            return Err(Error::Io(errno()));
        }
        let desired = if enabled {
            current | libc::O_NONBLOCK
        } else {
            current & !libc::O_NONBLOCK
        };
        // SAFETY: `fcntl` updates flags on a valid fd.
        let result = unsafe { libc::fcntl(fd, libc::F_SETFL, desired) };
        if result == -1 {
            Err(Error::Io(errno()))
        } else {
            Ok(())
        }
    }

    fn recv_native(fd: RawFd) -> Result<Option<CanFrame>> {
        let mut fd_frame: CanFdFrameNative = unsafe { zeroed() };
        // SAFETY: the buffer is valid for writes and sized to the maximum frame struct.
        let received = unsafe {
            libc::recv(
                fd,
                &mut fd_frame as *mut _ as *mut c_void,
                size_of::<CanFdFrameNative>(),
                0,
            )
        };
        if received == -1 {
            let error = errno();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(Error::Io(error));
        }

        let (fd_mode, len, flags) = if received as usize == size_of::<CanFdFrameNative>() {
            (true, usize::from(fd_frame.len), fd_frame.flags)
        } else {
            (false, usize::from(fd_frame.len.min(8)), 0)
        };

        let raw_id = fd_frame.can_id;
        let id = if raw_id & CAN_EFF_FLAG != 0 {
            CanId::extended(raw_id & CAN_EFF_MASK)?
        } else {
            CanId::standard((raw_id & 0x7ff) as u16)?
        };
        let class = if raw_id & CAN_ERR_FLAG != 0 {
            FrameClass::Error
        } else if raw_id & CAN_RTR_FLAG != 0 {
            FrameClass::Remote
        } else {
            FrameClass::Data
        };

        Ok(Some(CanFrame::new(
            id,
            class,
            fd_frame.data[..len.min(fd_frame.data.len())].to_vec(),
            fd_mode,
            FdFlags {
                bit_rate_switch: flags & CANFD_BRS != 0,
                error_state_indicator: flags & CANFD_ESI != 0,
            },
        )?))
    }

    fn open_socket(protocol: c_int) -> Result<RawFd> {
        // SAFETY: `socket` is invoked with valid constants and returns an owned fd on success.
        let fd = unsafe { libc::socket(libc::PF_CAN, libc::SOCK_RAW, protocol) };
        if fd == -1 {
            Err(Error::Io(errno()))
        } else {
            Ok(fd)
        }
    }

    fn close_fd(fd: RawFd) {
        // SAFETY: closing an owned fd is safe; errors are intentionally ignored in Drop paths.
        unsafe {
            libc::close(fd);
        }
    }

    pub struct RawSocketImpl {
        fd: RawFd,
        interface: String,
        ifindex: u32,
    }

    impl RawSocketImpl {
        pub fn open(interface: &str) -> Result<Self> {
            let fd = open_socket(CAN_RAW)?;
            if let Err(error) = set_fd_frames(fd).and_then(|_| bind_raw_socket(fd, interface)) {
                close_fd(fd);
                return Err(error);
            }

            Ok(Self {
                fd,
                interface: interface.to_string(),
                ifindex: interface_index(interface)?,
            })
        }

        pub fn set_nonblocking(&self, enabled: bool) -> Result<()> {
            set_nonblocking(self.fd, enabled)
        }

        pub fn send(&self, frame: &CanFrame) -> Result<()> {
            if frame.fd {
                let mut native = CanFdFrameNative {
                    can_id: can_id_from_frame(frame),
                    len: frame.data.len() as u8,
                    flags: 0,
                    res0: 0,
                    res1: 0,
                    data: [0; 64],
                };
                native.data[..frame.data.len()].copy_from_slice(&frame.data);
                if frame.fd_flags.bit_rate_switch {
                    native.flags |= CANFD_BRS;
                }
                if frame.fd_flags.error_state_indicator {
                    native.flags |= CANFD_ESI;
                }
                // SAFETY: the native frame is a fully initialized buffer with the correct size.
                let written = unsafe {
                    libc::write(
                        self.fd,
                        &native as *const _ as *const c_void,
                        size_of::<CanFdFrameNative>(),
                    )
                };
                if written == -1 {
                    return Err(Error::Io(errno()));
                }
            } else {
                let mut native = CanFrameNative {
                    can_id: can_id_from_frame(frame),
                    can_dlc: frame.data.len() as u8,
                    len8_dlc: 0,
                    pad: 0,
                    res0: 0,
                    data: [0; 8],
                };
                native.data[..frame.data.len()].copy_from_slice(&frame.data);
                // SAFETY: the classic frame is fully initialized and the size matches the kernel struct.
                let written = unsafe {
                    libc::write(
                        self.fd,
                        &native as *const _ as *const c_void,
                        size_of::<CanFrameNative>(),
                    )
                };
                if written == -1 {
                    return Err(Error::Io(errno()));
                }
            }

            Ok(())
        }

        pub fn recv(&self) -> Result<Option<CanFrame>> {
            recv_native(self.fd)
        }

        pub fn recv_event(&self) -> Result<Option<CaptureEvent>> {
            let Some(frame) = self.recv()? else {
                return Ok(None);
            };
            Ok(Some(CaptureEvent::new(
                Timestamp::from_system_time(SystemTime::now())?,
                InterfaceRef {
                    name: Some(self.interface.clone()),
                    index: Some(self.ifindex),
                },
                Direction::Rx,
                frame,
            )))
        }
    }

    impl Drop for RawSocketImpl {
        fn drop(&mut self) {
            close_fd(self.fd);
        }
    }

    pub struct IsotpSocketImpl {
        fd: RawFd,
    }

    impl IsotpSocketImpl {
        pub fn open(interface: &str, tx_id: u32, rx_id: u32) -> Result<Self> {
            let fd = open_socket(CAN_ISOTP)?;
            let ifindex = interface_index(interface)? as c_int;
            let addr = SockAddrCan {
                can_family: libc::AF_CAN as libc::sa_family_t,
                can_ifindex: ifindex,
                can_addr: CanAddr {
                    tp: CanTpAddr { rx_id, tx_id },
                },
            };
            // SAFETY: `addr` is a valid sockaddr_can with ISO-TP transport addresses.
            let result = unsafe {
                libc::bind(
                    fd,
                    &addr as *const _ as *const libc::sockaddr,
                    size_of::<SockAddrCan>() as libc::socklen_t,
                )
            };
            if result == -1 {
                close_fd(fd);
                return Err(Error::Io(errno()));
            }
            Ok(Self { fd })
        }

        pub fn send(&self, payload: &[u8]) -> Result<()> {
            // SAFETY: the payload slice is valid for reads for its full length.
            let written =
                unsafe { libc::write(self.fd, payload.as_ptr() as *const c_void, payload.len()) };
            if written == -1 {
                Err(Error::Io(errno()))
            } else {
                Ok(())
            }
        }

        pub fn recv(&self, buffer: &mut [u8]) -> Result<usize> {
            // SAFETY: the destination buffer is valid for writes.
            let read =
                unsafe { libc::read(self.fd, buffer.as_mut_ptr() as *mut c_void, buffer.len()) };
            if read == -1 {
                Err(Error::Io(errno()))
            } else {
                Ok(read as usize)
            }
        }
    }

    impl Drop for IsotpSocketImpl {
        fn drop(&mut self) {
            close_fd(self.fd);
        }
    }

    pub struct J1939SocketImpl {
        fd: RawFd,
    }

    impl J1939SocketImpl {
        pub fn open(interface: &str, name: u64, pgn: u32, address: u8) -> Result<Self> {
            let fd = open_socket(CAN_J1939)?;
            let ifindex = interface_index(interface)? as c_int;
            let addr = SockAddrCan {
                can_family: libc::AF_CAN as libc::sa_family_t,
                can_ifindex: ifindex,
                can_addr: CanAddr {
                    j1939: CanJ1939Addr {
                        name,
                        pgn,
                        addr: address,
                    },
                },
            };
            // SAFETY: `addr` is a valid sockaddr_can with J1939 addressing.
            let result = unsafe {
                libc::bind(
                    fd,
                    &addr as *const _ as *const libc::sockaddr,
                    size_of::<SockAddrCan>() as libc::socklen_t,
                )
            };
            if result == -1 {
                close_fd(fd);
                return Err(Error::Io(errno()));
            }
            Ok(Self { fd })
        }

        pub fn send(&self, payload: &[u8]) -> Result<()> {
            // SAFETY: the payload slice is valid for reads for its full length.
            let written =
                unsafe { libc::write(self.fd, payload.as_ptr() as *const c_void, payload.len()) };
            if written == -1 {
                Err(Error::Io(errno()))
            } else {
                Ok(())
            }
        }

        pub fn recv(&self, buffer: &mut [u8]) -> Result<usize> {
            // SAFETY: the destination buffer is valid for writes.
            let read =
                unsafe { libc::read(self.fd, buffer.as_mut_ptr() as *mut c_void, buffer.len()) };
            if read == -1 {
                Err(Error::Io(errno()))
            } else {
                Ok(read as usize)
            }
        }
    }

    impl Drop for J1939SocketImpl {
        fn drop(&mut self) {
            close_fd(self.fd);
        }
    }

    pub fn interfaces() -> Result<Vec<InterfaceInfo>> {
        let mut out = Vec::new();
        // SAFETY: `if_nameindex` returns a null-terminated table owned by libc.
        let entries = unsafe { libc::if_nameindex() };
        if entries.is_null() {
            return Err(Error::Io(errno()));
        }

        let mut cursor = entries;
        loop {
            // SAFETY: `cursor` walks a table terminated by `{0, null}`.
            let entry = unsafe { *cursor };
            if entry.if_index == 0 || entry.if_name.is_null() {
                break;
            }
            // SAFETY: libc guarantees `if_name` points to a valid C string for each entry.
            let name = unsafe { std::ffi::CStr::from_ptr(entry.if_name) }
                .to_string_lossy()
                .into_owned();
            out.push(InterfaceInfo {
                name,
                index: entry.if_index,
            });
            // SAFETY: pointer arithmetic stays within the libc table until the sentinel entry.
            cursor = unsafe { cursor.add(1) };
        }

        // SAFETY: `entries` came from `if_nameindex` and must be freed once.
        unsafe {
            libc::if_freenameindex(entries);
        }
        Ok(out)
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use cantools_core::{CanFrame, CaptureEvent};

    use super::{Error, InterfaceInfo, Result};

    pub struct RawSocketImpl;
    pub struct IsotpSocketImpl;
    pub struct J1939SocketImpl;

    impl RawSocketImpl {
        pub fn open(_interface: &str) -> Result<Self> {
            Err(Error::UnsupportedPlatform)
        }

        pub fn set_nonblocking(&self, _enabled: bool) -> Result<()> {
            Err(Error::UnsupportedPlatform)
        }

        pub fn send(&self, _frame: &CanFrame) -> Result<()> {
            Err(Error::UnsupportedPlatform)
        }

        pub fn recv(&self) -> Result<Option<CanFrame>> {
            Err(Error::UnsupportedPlatform)
        }

        pub fn recv_event(&self) -> Result<Option<CaptureEvent>> {
            Err(Error::UnsupportedPlatform)
        }
    }

    impl IsotpSocketImpl {
        pub fn open(_interface: &str, _tx_id: u32, _rx_id: u32) -> Result<Self> {
            Err(Error::UnsupportedPlatform)
        }

        pub fn send(&self, _payload: &[u8]) -> Result<()> {
            Err(Error::UnsupportedPlatform)
        }

        pub fn recv(&self, _buffer: &mut [u8]) -> Result<usize> {
            Err(Error::UnsupportedPlatform)
        }
    }

    impl J1939SocketImpl {
        pub fn open(_interface: &str, _name: u64, _pgn: u32, _address: u8) -> Result<Self> {
            Err(Error::UnsupportedPlatform)
        }

        pub fn send(&self, _payload: &[u8]) -> Result<()> {
            Err(Error::UnsupportedPlatform)
        }

        pub fn recv(&self, _buffer: &mut [u8]) -> Result<usize> {
            Err(Error::UnsupportedPlatform)
        }
    }

    pub fn interfaces() -> Result<Vec<InterfaceInfo>> {
        Err(Error::UnsupportedPlatform)
    }
}
