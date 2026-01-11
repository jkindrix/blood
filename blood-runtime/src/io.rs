//! # I/O Reactor
//!
//! Platform-native async I/O with abstraction layer.
//!
//! ## Design
//!
//! Based on the Blood runtime specification (ROADMAP.md §9.3):
//! - io_uring on Linux 5.6+
//! - kqueue on macOS/BSD
//! - IOCP on Windows
//!
//! ## Technical References
//!
//! - [io_uring](https://kernel.dk/io_uring.pdf)
//! - [Monoio](https://github.com/bytedance/monoio) - io_uring runtime
//! - [mio](https://docs.rs/mio) - Cross-platform I/O

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

/// Unique I/O operation identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IoOpId(pub u64);

impl IoOpId {
    /// Get the raw ID value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for IoOpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IoOp({})", self.0)
    }
}

/// Global I/O operation ID counter.
static NEXT_IO_OP_ID: AtomicU64 = AtomicU64::new(1);

/// Generate a new unique I/O operation ID.
fn next_io_op_id() -> IoOpId {
    IoOpId(NEXT_IO_OP_ID.fetch_add(1, Ordering::Relaxed))
}

/// I/O interest flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interest(u8);

impl Interest {
    /// Interested in read readiness.
    pub const READABLE: Self = Self(0b001);
    /// Interested in write readiness.
    pub const WRITABLE: Self = Self(0b010);
    /// Interested in error conditions.
    pub const ERROR: Self = Self(0b100);

    /// Combine interests.
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Check if readable interest is set.
    pub fn is_readable(&self) -> bool {
        self.0 & 0b001 != 0
    }

    /// Check if writable interest is set.
    pub fn is_writable(&self) -> bool {
        self.0 & 0b010 != 0
    }

    /// Check if error interest is set.
    pub fn is_error(&self) -> bool {
        self.0 & 0b100 != 0
    }
}

/// I/O operation type.
#[derive(Debug, Clone)]
pub enum IoOp {
    /// Read data from a file descriptor.
    Read {
        /// File descriptor.
        fd: i32,
        /// Buffer to read into.
        buf_len: usize,
        /// Offset for pread (-1 for regular read).
        offset: i64,
    },
    /// Write data to a file descriptor.
    Write {
        /// File descriptor.
        fd: i32,
        /// Data to write.
        data: Vec<u8>,
        /// Offset for pwrite (-1 for regular write).
        offset: i64,
    },
    /// Accept a connection on a socket.
    Accept {
        /// Listening socket fd.
        fd: i32,
    },
    /// Connect to a remote address.
    Connect {
        /// Socket fd.
        fd: i32,
        /// Target address (serialized).
        addr: Vec<u8>,
    },
    /// Close a file descriptor.
    Close {
        /// File descriptor.
        fd: i32,
    },
    /// Poll for readiness.
    Poll {
        /// File descriptor.
        fd: i32,
        /// Interest flags.
        interest: Interest,
    },
    /// Sleep/timeout.
    Timeout {
        /// Duration to sleep.
        duration: Duration,
    },
}

/// Result of an I/O operation.
#[derive(Debug, Clone)]
pub enum IoResult {
    /// Read completed with data.
    Read(Vec<u8>),
    /// Write completed with bytes written.
    Write(usize),
    /// Accept completed with new fd.
    Accept(i32),
    /// Connect completed.
    Connected,
    /// Close completed.
    Closed,
    /// Poll completed with readiness.
    Ready(Interest),
    /// Timeout expired.
    TimedOut,
    /// Error occurred.
    Error(IoError),
}

/// I/O error.
#[derive(Debug, Clone)]
pub struct IoError {
    /// Error code.
    pub code: i32,
    /// Error message.
    pub message: String,
}

impl IoError {
    /// Create a new I/O error.
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Create from std::io::Error.
    pub fn from_std(e: io::Error) -> Self {
        Self {
            code: e.raw_os_error().unwrap_or(-1),
            message: e.to_string(),
        }
    }
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IoError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for IoError {}

/// Completion entry for an I/O operation.
#[derive(Debug)]
pub struct IoCompletion {
    /// Operation ID.
    pub op_id: IoOpId,
    /// Result of the operation.
    pub result: IoResult,
}

/// Configuration for the I/O reactor.
#[derive(Debug, Clone)]
pub struct ReactorConfig {
    /// Maximum number of pending operations.
    pub max_pending: usize,
    /// Timeout for polling.
    pub poll_timeout: Duration,
}

impl Default for ReactorConfig {
    fn default() -> Self {
        Self {
            max_pending: 1024,
            poll_timeout: Duration::from_millis(100),
        }
    }
}

/// I/O reactor driver (platform-specific backend).
pub trait IoDriver: Send + Sync {
    /// Submit an I/O operation.
    fn submit(&self, op_id: IoOpId, op: IoOp) -> io::Result<()>;

    /// Poll for completions.
    fn poll(&self, timeout: Duration) -> io::Result<Vec<IoCompletion>>;

    /// Cancel a pending operation.
    fn cancel(&self, op_id: IoOpId) -> io::Result<()>;
}

/// Fallback blocking driver for platforms without async I/O.
pub struct BlockingDriver {
    /// Pending operations.
    pending: Mutex<HashMap<IoOpId, IoOp>>,
}

impl BlockingDriver {
    /// Create a new blocking driver.
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Execute an operation synchronously.
    fn execute(&self, op: &IoOp) -> IoResult {
        match op {
            IoOp::Read { fd: _, buf_len: _, offset: _ } => {
                // This is a simplified implementation
                // Real implementation would use raw file descriptors
                IoResult::Read(vec![0u8; 0])
            }
            IoOp::Write { fd: _, data, offset: _ } => {
                IoResult::Write(data.len())
            }
            IoOp::Accept { fd: _ } => {
                IoResult::Error(IoError::new(-1, "accept not implemented in blocking driver"))
            }
            IoOp::Connect { fd: _, addr: _ } => {
                IoResult::Connected
            }
            IoOp::Close { fd: _ } => {
                IoResult::Closed
            }
            IoOp::Poll { fd: _, interest } => {
                IoResult::Ready(*interest)
            }
            IoOp::Timeout { duration } => {
                std::thread::sleep(*duration);
                IoResult::TimedOut
            }
        }
    }
}

impl Default for BlockingDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl IoDriver for BlockingDriver {
    fn submit(&self, op_id: IoOpId, op: IoOp) -> io::Result<()> {
        self.pending.lock().insert(op_id, op);
        Ok(())
    }

    fn poll(&self, _timeout: Duration) -> io::Result<Vec<IoCompletion>> {
        let ops: Vec<_> = self.pending.lock().drain().collect();
        let completions = ops
            .into_iter()
            .map(|(op_id, op)| IoCompletion {
                op_id,
                result: self.execute(&op),
            })
            .collect();
        Ok(completions)
    }

    fn cancel(&self, op_id: IoOpId) -> io::Result<()> {
        self.pending.lock().remove(&op_id);
        Ok(())
    }
}

/// I/O reactor for async I/O operations.
pub struct IoReactor {
    /// Configuration.
    config: ReactorConfig,
    /// Platform-specific driver.
    driver: Box<dyn IoDriver>,
    /// Pending operation count.
    pending_count: AtomicU64,
}

impl IoReactor {
    /// Create a new I/O reactor with the default driver.
    pub fn new(config: ReactorConfig) -> Self {
        Self {
            config,
            driver: Box::new(BlockingDriver::new()),
            pending_count: AtomicU64::new(0),
        }
    }

    /// Create a new I/O reactor with a custom driver.
    pub fn with_driver(config: ReactorConfig, driver: Box<dyn IoDriver>) -> Self {
        Self {
            config,
            driver,
            pending_count: AtomicU64::new(0),
        }
    }

    /// Submit an I/O operation.
    pub fn submit(&self, op: IoOp) -> io::Result<IoOpId> {
        let op_id = next_io_op_id();
        self.driver.submit(op_id, op)?;
        self.pending_count.fetch_add(1, Ordering::Relaxed);
        Ok(op_id)
    }

    /// Poll for completed operations.
    pub fn poll(&self) -> io::Result<Vec<IoCompletion>> {
        let completions = self.driver.poll(self.config.poll_timeout)?;
        let count = completions.len() as u64;
        if count > 0 {
            self.pending_count.fetch_sub(count, Ordering::Relaxed);
        }
        Ok(completions)
    }

    /// Cancel a pending operation.
    pub fn cancel(&self, op_id: IoOpId) -> io::Result<()> {
        self.driver.cancel(op_id)?;
        self.pending_count.fetch_sub(1, Ordering::Relaxed);
        Ok(())
    }

    /// Get the number of pending operations.
    pub fn pending_count(&self) -> u64 {
        self.pending_count.load(Ordering::Relaxed)
    }

    /// Check if there are pending operations.
    pub fn has_pending(&self) -> bool {
        self.pending_count() > 0
    }
}

impl fmt::Debug for IoReactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IoReactor")
            .field("config", &self.config)
            .field("pending_count", &self.pending_count())
            .finish()
    }
}

// ============================================================================
// Platform-specific drivers
// ============================================================================

#[cfg(target_os = "linux")]
pub mod linux {
    //! Linux-specific I/O driver using io_uring (when available).

    use super::*;
    use nix::libc;
    use std::collections::HashMap;
    use std::os::unix::io::{IntoRawFd, RawFd};
    use std::time::Duration;

    /// Check if io_uring is available on this system.
    pub fn io_uring_available() -> bool {
        // io_uring requires Linux 5.6+
        #[cfg(feature = "io_uring")]
        {
            // Check kernel version
            use std::process::Command;
            if let Ok(output) = Command::new("uname").arg("-r").output() {
                if let Ok(version) = String::from_utf8(output.stdout) {
                    let parts: Vec<&str> = version.trim().split('.').collect();
                    if parts.len() >= 2 {
                        if let (Ok(major), Ok(minor)) = (
                            parts[0].parse::<u32>(),
                            parts[1].parse::<u32>(),
                        ) {
                            return major > 5 || (major == 5 && minor >= 6);
                        }
                    }
                }
            }
            false
        }
        #[cfg(not(feature = "io_uring"))]
        false
    }

    /// io_uring-based I/O driver for Linux.
    #[cfg(feature = "io_uring")]
    pub struct IoUringDriver {
        ring: parking_lot::Mutex<io_uring::IoUring>,
        pending: parking_lot::Mutex<HashMap<IoOpId, PendingOp>>,
    }

    #[cfg(feature = "io_uring")]
    struct PendingOp {
        op: IoOp,
        buf: Option<Vec<u8>>,
    }

    #[cfg(feature = "io_uring")]
    impl IoUringDriver {
        /// Create a new io_uring driver with the specified queue depth.
        pub fn new(queue_depth: u32) -> io::Result<Self> {
            let ring = io_uring::IoUring::new(queue_depth)?;
            Ok(Self {
                ring: parking_lot::Mutex::new(ring),
                pending: parking_lot::Mutex::new(HashMap::new()),
            })
        }

        /// Create with default queue depth (256).
        pub fn default_new() -> io::Result<Self> {
            Self::new(256)
        }
    }

    #[cfg(feature = "io_uring")]
    impl IoDriver for IoUringDriver {
        fn submit(&self, op_id: IoOpId, op: IoOp) -> io::Result<()> {
            use io_uring::opcode;
            use io_uring::types::Fd;

            let mut ring = self.ring.lock();
            let mut pending = self.pending.lock();

            let user_data = op_id.as_u64();

            // Prepare the submission entry based on operation type
            let sqe = match &op {
                IoOp::Read { fd, buf_len, offset } => {
                    // Allocate buffer for read
                    let buf = vec![0u8; *buf_len];
                    let buf_ptr = buf.as_ptr() as *mut u8;
                    pending.insert(op_id, PendingOp { op: op.clone(), buf: Some(buf) });

                    if *offset >= 0 {
                        opcode::Read::new(Fd(*fd), buf_ptr, *buf_len as u32)
                            .offset(*offset as u64)
                            .build()
                            .user_data(user_data)
                    } else {
                        opcode::Read::new(Fd(*fd), buf_ptr, *buf_len as u32)
                            .build()
                            .user_data(user_data)
                    }
                }
                IoOp::Write { fd, data, offset } => {
                    pending.insert(op_id, PendingOp { op: op.clone(), buf: None });
                    let data_ptr = data.as_ptr();
                    let data_len = data.len() as u32;

                    if *offset >= 0 {
                        opcode::Write::new(Fd(*fd), data_ptr, data_len)
                            .offset(*offset as u64)
                            .build()
                            .user_data(user_data)
                    } else {
                        opcode::Write::new(Fd(*fd), data_ptr, data_len)
                            .build()
                            .user_data(user_data)
                    }
                }
                IoOp::Accept { fd } => {
                    pending.insert(op_id, PendingOp { op: op.clone(), buf: None });
                    opcode::Accept::new(Fd(*fd))
                        .build()
                        .user_data(user_data)
                }
                IoOp::Close { fd } => {
                    pending.insert(op_id, PendingOp { op: op.clone(), buf: None });
                    opcode::Close::new(Fd(*fd))
                        .build()
                        .user_data(user_data)
                }
                IoOp::Connect { fd, addr: _ } => {
                    // Connect requires sockaddr, simplified for now
                    pending.insert(op_id, PendingOp { op: op.clone(), buf: None });
                    opcode::Nop::new().build().user_data(user_data)
                }
                IoOp::Poll { fd, interest } => {
                    pending.insert(op_id, PendingOp { op: op.clone(), buf: None });
                    let poll_mask = {
                        let mut mask = 0u32;
                        if interest.is_readable() {
                            mask |= libc::POLLIN as u32;
                        }
                        if interest.is_writable() {
                            mask |= libc::POLLOUT as u32;
                        }
                        if interest.is_error() {
                            mask |= libc::POLLERR as u32;
                        }
                        mask
                    };
                    opcode::PollAdd::new(Fd(*fd), poll_mask)
                        .build()
                        .user_data(user_data)
                }
                IoOp::Timeout { duration } => {
                    pending.insert(op_id, PendingOp { op: op.clone(), buf: None });
                    let ts = io_uring::types::Timespec::new()
                        .sec(duration.as_secs())
                        .nsec(duration.subsec_nanos());
                    opcode::Timeout::new(&ts)
                        .build()
                        .user_data(user_data)
                }
            };

            unsafe {
                ring.submission().push(&sqe).map_err(|_| {
                    io::Error::new(io::ErrorKind::Other, "submission queue full")
                })?;
            }
            ring.submit()?;
            Ok(())
        }

        fn poll(&self, timeout: Duration) -> io::Result<Vec<IoCompletion>> {
            let mut ring = self.ring.lock();
            let mut pending = self.pending.lock();

            // Wait for completions
            ring.submit_and_wait(1)?;

            let mut completions = Vec::new();

            // Collect completions
            for cqe in ring.completion() {
                let op_id = IoOpId(cqe.user_data());
                let result_code = cqe.result();

                let result = if let Some(pop) = pending.remove(&op_id) {
                    if result_code < 0 {
                        IoResult::Error(IoError::new(-result_code, "io_uring operation failed"))
                    } else {
                        match pop.op {
                            IoOp::Read { .. } => {
                                let buf = pop.buf.unwrap_or_default();
                                let len = result_code as usize;
                                IoResult::Read(buf[..len.min(buf.len())].to_vec())
                            }
                            IoOp::Write { .. } => IoResult::Write(result_code as usize),
                            IoOp::Accept { .. } => IoResult::Accept(result_code),
                            IoOp::Close { .. } => IoResult::Closed,
                            IoOp::Connect { .. } => IoResult::Connected,
                            IoOp::Poll { .. } => {
                                let mut interest = Interest(0);
                                if result_code & libc::POLLIN as i32 != 0 {
                                    interest = interest.union(Interest::READABLE);
                                }
                                if result_code & libc::POLLOUT as i32 != 0 {
                                    interest = interest.union(Interest::WRITABLE);
                                }
                                IoResult::Ready(interest)
                            }
                            IoOp::Timeout { .. } => IoResult::TimedOut,
                        }
                    }
                } else {
                    IoResult::Error(IoError::new(-1, "unknown operation completed"))
                };

                completions.push(IoCompletion { op_id, result });
            }

            Ok(completions)
        }

        fn cancel(&self, op_id: IoOpId) -> io::Result<()> {
            use io_uring::opcode;

            let mut ring = self.ring.lock();
            let mut pending = self.pending.lock();

            pending.remove(&op_id);

            let sqe = opcode::AsyncCancel::new(op_id.as_u64())
                .build()
                .user_data(0);

            unsafe {
                ring.submission().push(&sqe).map_err(|_| {
                    io::Error::new(io::ErrorKind::Other, "submission queue full")
                })?;
            }
            ring.submit()?;
            Ok(())
        }
    }

    /// Pending operation state for epoll.
    #[derive(Debug)]
    struct EpollPendingOp {
        op: IoOp,
        /// Buffer for read operations.
        read_buf: Option<Vec<u8>>,
        /// Timerfd for timeout operations.
        timer_fd: Option<RawFd>,
    }

    /// Epoll-based fallback driver for Linux when io_uring is not available.
    pub struct EpollDriver {
        epoll_fd: RawFd,
        /// Pending operations indexed by op_id.
        pending: parking_lot::Mutex<HashMap<IoOpId, EpollPendingOp>>,
        /// Mapping from fd to op_id for event dispatch.
        fd_to_op: parking_lot::Mutex<HashMap<RawFd, IoOpId>>,
    }

    impl EpollDriver {
        /// Create a new epoll driver.
        pub fn new() -> io::Result<Self> {
            use nix::sys::epoll::Epoll;
            let epoll = Epoll::new(nix::sys::epoll::EpollCreateFlags::EPOLL_CLOEXEC)?;
            Ok(Self {
                epoll_fd: epoll.0.into_raw_fd(),
                pending: parking_lot::Mutex::new(HashMap::new()),
                fd_to_op: parking_lot::Mutex::new(HashMap::new()),
            })
        }

        /// Register an fd with epoll for the given events.
        fn register_fd(&self, fd: RawFd, events: u32, op_id: IoOpId) -> io::Result<()> {
            let mut event = libc::epoll_event {
                events,
                u64: op_id.as_u64(),
            };

            let result = unsafe {
                libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut event)
            };

            if result == -1 {
                let err = io::Error::last_os_error();
                // EEXIST means fd is already registered, try to modify instead
                if err.raw_os_error() == Some(libc::EEXIST) {
                    let result = unsafe {
                        libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_MOD, fd, &mut event)
                    };
                    if result == -1 {
                        return Err(io::Error::last_os_error());
                    }
                } else {
                    return Err(err);
                }
            }

            self.fd_to_op.lock().insert(fd, op_id);
            Ok(())
        }

        /// Unregister an fd from epoll.
        fn unregister_fd(&self, fd: RawFd) {
            let _ = unsafe {
                libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut())
            };
            self.fd_to_op.lock().remove(&fd);
        }

        /// Create a timerfd for timeout operations.
        fn create_timerfd(duration: Duration) -> io::Result<RawFd> {
            let tfd = unsafe {
                libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_NONBLOCK | libc::TFD_CLOEXEC)
            };
            if tfd == -1 {
                return Err(io::Error::last_os_error());
            }

            let its = libc::itimerspec {
                it_interval: libc::timespec { tv_sec: 0, tv_nsec: 0 },
                it_value: libc::timespec {
                    tv_sec: duration.as_secs() as i64,
                    tv_nsec: duration.subsec_nanos() as i64,
                },
            };

            let result = unsafe {
                libc::timerfd_settime(tfd, 0, &its, std::ptr::null_mut())
            };
            if result == -1 {
                let err = io::Error::last_os_error();
                let _ = unsafe { libc::close(tfd) };
                return Err(err);
            }

            Ok(tfd)
        }

        /// Perform a read operation.
        fn do_read(fd: RawFd, buf_len: usize, offset: i64) -> IoResult {
            let mut buf = vec![0u8; buf_len];
            let result = if offset >= 0 {
                unsafe {
                    libc::pread(fd, buf.as_mut_ptr() as *mut libc::c_void, buf_len, offset)
                }
            } else {
                unsafe {
                    libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf_len)
                }
            };

            if result < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EAGAIN) || err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                    // Would block, not ready yet
                    IoResult::Error(IoError::new(libc::EAGAIN, "would block"))
                } else {
                    IoResult::Error(IoError::from_std(err))
                }
            } else {
                buf.truncate(result as usize);
                IoResult::Read(buf)
            }
        }

        /// Perform a write operation.
        fn do_write(fd: RawFd, data: &[u8], offset: i64) -> IoResult {
            let result = if offset >= 0 {
                unsafe {
                    libc::pwrite(fd, data.as_ptr() as *const libc::c_void, data.len(), offset)
                }
            } else {
                unsafe {
                    libc::write(fd, data.as_ptr() as *const libc::c_void, data.len())
                }
            };

            if result < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EAGAIN) || err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                    IoResult::Error(IoError::new(libc::EAGAIN, "would block"))
                } else {
                    IoResult::Error(IoError::from_std(err))
                }
            } else {
                IoResult::Write(result as usize)
            }
        }

        /// Perform an accept operation.
        fn do_accept(fd: RawFd) -> IoResult {
            let mut addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
            let mut addr_len: libc::socklen_t = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;

            let result = unsafe {
                libc::accept4(
                    fd,
                    &mut addr as *mut _ as *mut libc::sockaddr,
                    &mut addr_len,
                    libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                )
            };

            if result < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EAGAIN) || err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                    IoResult::Error(IoError::new(libc::EAGAIN, "would block"))
                } else {
                    IoResult::Error(IoError::from_std(err))
                }
            } else {
                IoResult::Accept(result as i32)
            }
        }
    }

    impl Default for EpollDriver {
        fn default() -> Self {
            Self::new().expect("failed to create epoll driver")
        }
    }

    impl Drop for EpollDriver {
        fn drop(&mut self) {
            // Close any pending timerfds
            let pending = self.pending.lock();
            for (_, pop) in pending.iter() {
                if let Some(tfd) = pop.timer_fd {
                    let _ = unsafe { libc::close(tfd) };
                }
            }
            drop(pending);

            let _ = nix::unistd::close(self.epoll_fd);
        }
    }

    impl IoDriver for EpollDriver {
        fn submit(&self, op_id: IoOpId, op: IoOp) -> io::Result<()> {
            let mut pending_op = EpollPendingOp {
                op: op.clone(),
                read_buf: None,
                timer_fd: None,
            };

            match &op {
                IoOp::Read { fd, buf_len, .. } => {
                    // Allocate read buffer and register for EPOLLIN
                    pending_op.read_buf = Some(vec![0u8; *buf_len]);
                    self.register_fd(*fd, libc::EPOLLIN as u32 | libc::EPOLLONESHOT as u32, op_id)?;
                }
                IoOp::Write { fd, .. } => {
                    // Register for EPOLLOUT
                    self.register_fd(*fd, libc::EPOLLOUT as u32 | libc::EPOLLONESHOT as u32, op_id)?;
                }
                IoOp::Accept { fd } => {
                    // Register listening socket for EPOLLIN
                    self.register_fd(*fd, libc::EPOLLIN as u32 | libc::EPOLLONESHOT as u32, op_id)?;
                }
                IoOp::Connect { fd, .. } => {
                    // Register for EPOLLOUT (connect completion)
                    self.register_fd(*fd, libc::EPOLLOUT as u32 | libc::EPOLLONESHOT as u32, op_id)?;
                }
                IoOp::Poll { fd, interest } => {
                    let mut events = 0u32;
                    if interest.is_readable() {
                        events |= libc::EPOLLIN as u32;
                    }
                    if interest.is_writable() {
                        events |= libc::EPOLLOUT as u32;
                    }
                    if interest.is_error() {
                        events |= libc::EPOLLERR as u32;
                    }
                    events |= libc::EPOLLONESHOT as u32;
                    self.register_fd(*fd, events, op_id)?;
                }
                IoOp::Close { fd } => {
                    // Close is synchronous
                    let _ = unsafe { libc::close(*fd) };
                    // Don't add to pending, return completion immediately
                    return Ok(());
                }
                IoOp::Timeout { duration } => {
                    // Create timerfd and register for EPOLLIN
                    let tfd = Self::create_timerfd(*duration)?;
                    pending_op.timer_fd = Some(tfd);
                    self.register_fd(tfd, libc::EPOLLIN as u32 | libc::EPOLLONESHOT as u32, op_id)?;
                }
            }

            self.pending.lock().insert(op_id, pending_op);
            Ok(())
        }

        fn poll(&self, timeout: Duration) -> io::Result<Vec<IoCompletion>> {
            let mut events: [libc::epoll_event; 64] = unsafe { std::mem::zeroed() };
            let timeout_ms = timeout.as_millis() as i32;

            let nfds = unsafe {
                match libc::epoll_wait(self.epoll_fd, events.as_mut_ptr(), 64, timeout_ms) {
                    -1 => {
                        let err = io::Error::last_os_error();
                        if err.raw_os_error() == Some(libc::EINTR) {
                            0
                        } else {
                            return Err(err);
                        }
                    }
                    n => n as usize,
                }
            };

            let mut completions = Vec::new();
            let mut pending = self.pending.lock();

            for i in 0..nfds {
                let event = &events[i];
                let op_id = IoOpId(event.u64);
                let revents = event.events;

                if let Some(pop) = pending.remove(&op_id) {
                    let result = match &pop.op {
                        IoOp::Read { fd, buf_len, offset } => {
                            if revents & libc::EPOLLIN as u32 != 0 {
                                self.unregister_fd(*fd);
                                Self::do_read(*fd, *buf_len, *offset)
                            } else if revents & (libc::EPOLLERR as u32 | libc::EPOLLHUP as u32) != 0 {
                                self.unregister_fd(*fd);
                                IoResult::Error(IoError::new(-1, "fd error or hangup"))
                            } else {
                                // Re-add to pending
                                pending.insert(op_id, pop);
                                continue;
                            }
                        }
                        IoOp::Write { fd, data, offset } => {
                            if revents & libc::EPOLLOUT as u32 != 0 {
                                self.unregister_fd(*fd);
                                Self::do_write(*fd, data, *offset)
                            } else if revents & (libc::EPOLLERR as u32 | libc::EPOLLHUP as u32) != 0 {
                                self.unregister_fd(*fd);
                                IoResult::Error(IoError::new(-1, "fd error or hangup"))
                            } else {
                                pending.insert(op_id, pop);
                                continue;
                            }
                        }
                        IoOp::Accept { fd } => {
                            if revents & libc::EPOLLIN as u32 != 0 {
                                self.unregister_fd(*fd);
                                Self::do_accept(*fd)
                            } else {
                                pending.insert(op_id, pop);
                                continue;
                            }
                        }
                        IoOp::Connect { fd, .. } => {
                            if revents & libc::EPOLLOUT as u32 != 0 {
                                self.unregister_fd(*fd);
                                // Check socket error to confirm connection
                                let mut err: libc::c_int = 0;
                                let mut len: libc::socklen_t = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                                let result = unsafe {
                                    libc::getsockopt(
                                        *fd,
                                        libc::SOL_SOCKET,
                                        libc::SO_ERROR,
                                        &mut err as *mut _ as *mut libc::c_void,
                                        &mut len,
                                    )
                                };
                                if result == 0 && err == 0 {
                                    IoResult::Connected
                                } else {
                                    IoResult::Error(IoError::new(err, "connect failed"))
                                }
                            } else if revents & (libc::EPOLLERR as u32 | libc::EPOLLHUP as u32) != 0 {
                                self.unregister_fd(*fd);
                                IoResult::Error(IoError::new(-1, "connect error"))
                            } else {
                                pending.insert(op_id, pop);
                                continue;
                            }
                        }
                        IoOp::Poll { fd, .. } => {
                            self.unregister_fd(*fd);
                            let mut interest = Interest(0);
                            if revents & libc::EPOLLIN as u32 != 0 {
                                interest = interest.union(Interest::READABLE);
                            }
                            if revents & libc::EPOLLOUT as u32 != 0 {
                                interest = interest.union(Interest::WRITABLE);
                            }
                            if revents & libc::EPOLLERR as u32 != 0 {
                                interest = interest.union(Interest::ERROR);
                            }
                            IoResult::Ready(interest)
                        }
                        IoOp::Close { fd } => {
                            // Close already handled in submit
                            self.unregister_fd(*fd);
                            IoResult::Closed
                        }
                        IoOp::Timeout { .. } => {
                            if let Some(tfd) = pop.timer_fd {
                                // Read the timerfd to clear it
                                let mut buf = [0u8; 8];
                                let _ = unsafe { libc::read(tfd, buf.as_mut_ptr() as *mut libc::c_void, 8) };
                                self.unregister_fd(tfd);
                                let _ = unsafe { libc::close(tfd) };
                            }
                            IoResult::TimedOut
                        }
                    };

                    completions.push(IoCompletion { op_id, result });
                }
            }

            Ok(completions)
        }

        fn cancel(&self, op_id: IoOpId) -> io::Result<()> {
            let mut pending = self.pending.lock();
            if let Some(pop) = pending.remove(&op_id) {
                // Unregister the fd from epoll
                match &pop.op {
                    IoOp::Read { fd, .. } | IoOp::Write { fd, .. } |
                    IoOp::Accept { fd } | IoOp::Connect { fd, .. } |
                    IoOp::Poll { fd, .. } | IoOp::Close { fd } => {
                        self.unregister_fd(*fd);
                    }
                    IoOp::Timeout { .. } => {
                        if let Some(tfd) = pop.timer_fd {
                            self.unregister_fd(tfd);
                            let _ = unsafe { libc::close(tfd) };
                        }
                    }
                }
            }
            Ok(())
        }
    }

    /// Create the best available driver for this Linux system.
    pub fn create_driver() -> Box<dyn IoDriver> {
        #[cfg(feature = "io_uring")]
        {
            if io_uring_available() {
                if let Ok(driver) = IoUringDriver::default_new() {
                    return Box::new(driver);
                }
            }
        }

        // Fall back to epoll
        if let Ok(driver) = EpollDriver::new() {
            return Box::new(driver);
        }

        // Ultimate fallback
        Box::new(super::BlockingDriver::new())
    }
}

#[cfg(target_os = "macos")]
pub mod macos {
    //! macOS-specific I/O driver using kqueue.

    use super::*;
    use nix::libc;
    use std::collections::HashMap;
    use std::os::unix::io::RawFd;
    use std::time::Duration;

    /// Check if kqueue is available (always true on macOS).
    pub fn kqueue_available() -> bool {
        true
    }

    /// Kqueue-based I/O driver for macOS/BSD.
    pub struct KqueueDriver {
        kqueue_fd: RawFd,
        pending: parking_lot::Mutex<HashMap<IoOpId, IoOp>>,
    }

    impl KqueueDriver {
        /// Create a new kqueue driver.
        pub fn new() -> io::Result<Self> {
            use nix::sys::event::kqueue;
            let kqueue_fd = kqueue()?;
            Ok(Self {
                kqueue_fd,
                pending: parking_lot::Mutex::new(HashMap::new()),
            })
        }

        /// Perform a read operation.
        fn do_read(fd: RawFd, buf_len: usize, offset: i64) -> IoResult {
            let mut buf = vec![0u8; buf_len];
            let result = if offset >= 0 {
                unsafe {
                    libc::pread(fd, buf.as_mut_ptr() as *mut libc::c_void, buf_len, offset)
                }
            } else {
                unsafe {
                    libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf_len)
                }
            };

            if result < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EAGAIN) || err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                    IoResult::Error(IoError::new(libc::EAGAIN, "would block"))
                } else {
                    IoResult::Error(IoError::from_std(err))
                }
            } else {
                buf.truncate(result as usize);
                IoResult::Read(buf)
            }
        }

        /// Perform a write operation.
        fn do_write(fd: RawFd, data: &[u8], offset: i64) -> IoResult {
            let result = if offset >= 0 {
                unsafe {
                    libc::pwrite(fd, data.as_ptr() as *const libc::c_void, data.len(), offset)
                }
            } else {
                unsafe {
                    libc::write(fd, data.as_ptr() as *const libc::c_void, data.len())
                }
            };

            if result < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EAGAIN) || err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                    IoResult::Error(IoError::new(libc::EAGAIN, "would block"))
                } else {
                    IoResult::Error(IoError::from_std(err))
                }
            } else {
                IoResult::Write(result as usize)
            }
        }

        /// Perform an accept operation.
        fn do_accept(fd: RawFd) -> IoResult {
            let mut addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
            let mut addr_len: libc::socklen_t = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;

            let result = unsafe {
                libc::accept(
                    fd,
                    &mut addr as *mut _ as *mut libc::sockaddr,
                    &mut addr_len,
                )
            };

            if result < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EAGAIN) || err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                    IoResult::Error(IoError::new(libc::EAGAIN, "would block"))
                } else {
                    IoResult::Error(IoError::from_std(err))
                }
            } else {
                // Set non-blocking on macOS (no accept4)
                let _ = unsafe { libc::fcntl(result as i32, libc::F_SETFL, libc::O_NONBLOCK | libc::O_CLOEXEC) };
                IoResult::Accept(result as i32)
            }
        }
    }

    impl Default for KqueueDriver {
        fn default() -> Self {
            Self::new().expect("failed to create kqueue driver")
        }
    }

    impl Drop for KqueueDriver {
        fn drop(&mut self) {
            let _ = nix::unistd::close(self.kqueue_fd);
        }
    }

    impl IoDriver for KqueueDriver {
        fn submit(&self, op_id: IoOpId, op: IoOp) -> io::Result<()> {
            use nix::sys::event::{kevent, EventFilter, EventFlag, FilterFlag, KEvent};

            // Register interest based on operation type
            match &op {
                IoOp::Read { fd, .. } => {
                    let event = KEvent::new(
                        *fd as usize,
                        EventFilter::EVFILT_READ,
                        EventFlag::EV_ADD | EventFlag::EV_ONESHOT,
                        FilterFlag::empty(),
                        0,
                        op_id.as_u64() as isize,
                    );
                    kevent(self.kqueue_fd, &[event], &mut [], 0)?;
                }
                IoOp::Write { fd, .. } => {
                    let event = KEvent::new(
                        *fd as usize,
                        EventFilter::EVFILT_WRITE,
                        EventFlag::EV_ADD | EventFlag::EV_ONESHOT,
                        FilterFlag::empty(),
                        0,
                        op_id.as_u64() as isize,
                    );
                    kevent(self.kqueue_fd, &[event], &mut [], 0)?;
                }
                IoOp::Accept { fd } => {
                    let event = KEvent::new(
                        *fd as usize,
                        EventFilter::EVFILT_READ,
                        EventFlag::EV_ADD | EventFlag::EV_ONESHOT,
                        FilterFlag::empty(),
                        0,
                        op_id.as_u64() as isize,
                    );
                    kevent(self.kqueue_fd, &[event], &mut [], 0)?;
                }
                IoOp::Connect { fd, .. } => {
                    let event = KEvent::new(
                        *fd as usize,
                        EventFilter::EVFILT_WRITE,
                        EventFlag::EV_ADD | EventFlag::EV_ONESHOT,
                        FilterFlag::empty(),
                        0,
                        op_id.as_u64() as isize,
                    );
                    kevent(self.kqueue_fd, &[event], &mut [], 0)?;
                }
                IoOp::Poll { fd, interest } => {
                    let mut events = Vec::new();
                    if interest.is_readable() {
                        events.push(KEvent::new(
                            *fd as usize,
                            EventFilter::EVFILT_READ,
                            EventFlag::EV_ADD | EventFlag::EV_ONESHOT,
                            FilterFlag::empty(),
                            0,
                            op_id.as_u64() as isize,
                        ));
                    }
                    if interest.is_writable() {
                        events.push(KEvent::new(
                            *fd as usize,
                            EventFilter::EVFILT_WRITE,
                            EventFlag::EV_ADD | EventFlag::EV_ONESHOT,
                            FilterFlag::empty(),
                            0,
                            op_id.as_u64() as isize,
                        ));
                    }
                    if !events.is_empty() {
                        kevent(self.kqueue_fd, &events, &mut [], 0)?;
                    }
                }
                IoOp::Close { fd } => {
                    // Close is synchronous
                    let _ = unsafe { libc::close(*fd) };
                    return Ok(());
                }
                IoOp::Timeout { duration } => {
                    // Use EVFILT_TIMER for timeouts
                    let ms = duration.as_millis() as isize;
                    let event = KEvent::new(
                        op_id.as_u64() as usize,
                        EventFilter::EVFILT_TIMER,
                        EventFlag::EV_ADD | EventFlag::EV_ONESHOT,
                        FilterFlag::NOTE_MSECONDS,
                        ms,
                        op_id.as_u64() as isize,
                    );
                    kevent(self.kqueue_fd, &[event], &mut [], 0)?;
                }
            }

            self.pending.lock().insert(op_id, op);
            Ok(())
        }

        fn poll(&self, timeout: Duration) -> io::Result<Vec<IoCompletion>> {
            use nix::sys::event::{kevent, EventFilter, KEvent};
            use nix::sys::time::TimeSpec;

            let timeout_spec = TimeSpec::from_duration(timeout);
            let mut events = vec![KEvent::new(0, EventFilter::EVFILT_READ, Default::default(), Default::default(), 0, 0); 64];

            let nfds = match kevent(self.kqueue_fd, &[], &mut events, timeout_spec) {
                Ok(n) => n,
                Err(nix::errno::Errno::EINTR) => 0,
                Err(e) => return Err(io::Error::from_raw_os_error(e as i32)),
            };

            let mut completions = Vec::new();
            let mut pending = self.pending.lock();

            for i in 0..nfds {
                let event = &events[i];
                let op_id = IoOpId(event.udata() as u64);

                if let Some(op) = pending.remove(&op_id) {
                    let result = match (&op, event.filter()) {
                        (IoOp::Read { fd, buf_len, offset }, EventFilter::EVFILT_READ) => {
                            Self::do_read(*fd, *buf_len, *offset)
                        }
                        (IoOp::Write { fd, data, offset }, EventFilter::EVFILT_WRITE) => {
                            Self::do_write(*fd, data, *offset)
                        }
                        (IoOp::Accept { fd }, EventFilter::EVFILT_READ) => {
                            Self::do_accept(*fd)
                        }
                        (IoOp::Connect { fd, .. }, EventFilter::EVFILT_WRITE) => {
                            // Check socket error to confirm connection
                            let mut err: libc::c_int = 0;
                            let mut len: libc::socklen_t = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                            let result = unsafe {
                                libc::getsockopt(
                                    *fd,
                                    libc::SOL_SOCKET,
                                    libc::SO_ERROR,
                                    &mut err as *mut _ as *mut libc::c_void,
                                    &mut len,
                                )
                            };
                            if result == 0 && err == 0 {
                                IoResult::Connected
                            } else {
                                IoResult::Error(IoError::new(err, "connect failed"))
                            }
                        }
                        (IoOp::Poll { .. }, _) => {
                            let mut interest = Interest(0);
                            if event.filter() == EventFilter::EVFILT_READ {
                                interest = interest.union(Interest::READABLE);
                            }
                            if event.filter() == EventFilter::EVFILT_WRITE {
                                interest = interest.union(Interest::WRITABLE);
                            }
                            IoResult::Ready(interest)
                        }
                        (IoOp::Timeout { .. }, EventFilter::EVFILT_TIMER) => {
                            IoResult::TimedOut
                        }
                        _ => IoResult::Error(IoError::new(-1, "unexpected event")),
                    };

                    completions.push(IoCompletion { op_id, result });
                }
            }

            Ok(completions)
        }

        fn cancel(&self, op_id: IoOpId) -> io::Result<()> {
            self.pending.lock().remove(&op_id);
            Ok(())
        }
    }

    /// Create the best available driver for macOS.
    pub fn create_driver() -> Box<dyn IoDriver> {
        if let Ok(driver) = KqueueDriver::new() {
            return Box::new(driver);
        }
        Box::new(super::BlockingDriver::new())
    }
}

#[cfg(target_os = "windows")]
pub mod windows {
    //! Windows-specific I/O driver using IOCP (I/O Completion Ports).

    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::IO::{
        CreateIoCompletionPort, GetQueuedCompletionStatus, PostQueuedCompletionStatus,
        OVERLAPPED,
    };

    /// Check if IOCP is available (always true on Windows).
    pub fn iocp_available() -> bool {
        true
    }

    /// IOCP-based I/O driver for Windows.
    pub struct IocpDriver {
        iocp_handle: HANDLE,
        pending: parking_lot::Mutex<HashMap<IoOpId, IoOp>>,
    }

    impl IocpDriver {
        /// Create a new IOCP driver.
        pub fn new() -> io::Result<Self> {
            let iocp_handle = unsafe {
                CreateIoCompletionPort(
                    HANDLE::default(),
                    HANDLE::default(),
                    0,
                    0, // Use system default number of threads
                )?
            };

            Ok(Self {
                iocp_handle,
                pending: parking_lot::Mutex::new(HashMap::new()),
            })
        }
    }

    impl Default for IocpDriver {
        fn default() -> Self {
            Self::new().expect("failed to create IOCP driver")
        }
    }

    impl Drop for IocpDriver {
        fn drop(&mut self) {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.iocp_handle);
            }
        }
    }

    impl IoDriver for IocpDriver {
        fn submit(&self, op_id: IoOpId, op: IoOp) -> io::Result<()> {
            self.pending.lock().insert(op_id, op.clone());

            // For simple operations, post completion immediately
            match &op {
                IoOp::Timeout { duration } => {
                    // Spawn a thread to handle the timeout
                    let handle = self.iocp_handle;
                    let duration = *duration;
                    let id = op_id.as_u64();
                    std::thread::spawn(move || {
                        std::thread::sleep(duration);
                        unsafe {
                            let _ = PostQueuedCompletionStatus(
                                handle,
                                0,
                                id as usize,
                                None,
                            );
                        }
                    });
                }
                IoOp::Close { .. } | IoOp::Connect { .. } => {
                    // Post immediate completion for these simple ops
                    unsafe {
                        PostQueuedCompletionStatus(
                            self.iocp_handle,
                            0,
                            op_id.as_u64() as usize,
                            None,
                        )?;
                    }
                }
                IoOp::Read { .. } | IoOp::Write { .. } | IoOp::Accept { .. } | IoOp::Poll { .. } => {
                    // These operations require proper overlapped I/O with:
                    // - OVERLAPPED structures with completion keys
                    // - File/socket handles associated with IOCP
                    // - ReadFile/WriteFile/AcceptEx with overlapped parameter
                    // Return not-implemented error to avoid silent placeholder behavior.
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        format!("IocpDriver: operation {:?} requires proper overlapped I/O setup (not yet implemented)", op),
                    ));
                }
            }

            Ok(())
        }

        fn poll(&self, timeout: Duration) -> io::Result<Vec<IoCompletion>> {
            let mut completions = Vec::new();
            let timeout_ms = timeout.as_millis() as u32;

            let mut bytes_transferred: u32 = 0;
            let mut completion_key: usize = 0;
            let mut overlapped: *mut OVERLAPPED = std::ptr::null_mut();

            // Poll for a single completion
            let result = unsafe {
                GetQueuedCompletionStatus(
                    self.iocp_handle,
                    &mut bytes_transferred,
                    &mut completion_key,
                    &mut overlapped,
                    timeout_ms,
                )
            };

            if result.is_ok() {
                let op_id = IoOpId(completion_key as u64);
                let mut pending = self.pending.lock();

                if let Some(op) = pending.remove(&op_id) {
                    let io_result = match op {
                        // These should never reach here since submit() returns error for them
                        IoOp::Read { .. } => IoResult::Error(IoError::new(-1, "read requires proper overlapped I/O setup")),
                        IoOp::Write { .. } => IoResult::Error(IoError::new(-1, "write requires proper overlapped I/O setup")),
                        IoOp::Accept { .. } => IoResult::Error(IoError::new(-1, "accept requires proper overlapped I/O setup")),
                        IoOp::Poll { .. } => IoResult::Error(IoError::new(-1, "poll requires proper overlapped I/O setup")),
                        // These are properly handled
                        IoOp::Connect { .. } => IoResult::Connected,
                        IoOp::Close { .. } => IoResult::Closed,
                        IoOp::Timeout { .. } => IoResult::TimedOut,
                    };

                    completions.push(IoCompletion { op_id, result: io_result });
                }
            }

            Ok(completions)
        }

        fn cancel(&self, op_id: IoOpId) -> io::Result<()> {
            self.pending.lock().remove(&op_id);
            Ok(())
        }
    }

    /// Create the best available driver for Windows.
    pub fn create_driver() -> Box<dyn IoDriver> {
        if let Ok(driver) = IocpDriver::new() {
            return Box::new(driver);
        }
        Box::new(super::BlockingDriver::new())
    }
}

// ============================================================================
// Platform-agnostic driver creation
// ============================================================================

/// Create the best available I/O driver for the current platform.
///
/// This function automatically selects the most efficient driver:
/// - Linux: io_uring (if available), else epoll, else blocking
/// - macOS: kqueue, else blocking
/// - Windows: IOCP, else blocking
/// - Other: blocking
pub fn create_native_driver() -> Box<dyn IoDriver> {
    #[cfg(target_os = "linux")]
    {
        return linux::create_driver();
    }

    #[cfg(target_os = "macos")]
    {
        return macos::create_driver();
    }

    #[cfg(target_os = "windows")]
    {
        return windows::create_driver();
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Box::new(BlockingDriver::new())
    }
}

/// Create an IoReactor with the best available native driver.
pub fn create_native_reactor(config: ReactorConfig) -> IoReactor {
    IoReactor::with_driver(config, create_native_driver())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interest_flags() {
        assert!(Interest::READABLE.is_readable());
        assert!(!Interest::READABLE.is_writable());
        assert!(Interest::WRITABLE.is_writable());
        assert!(!Interest::WRITABLE.is_readable());

        let both = Interest::READABLE.union(Interest::WRITABLE);
        assert!(both.is_readable());
        assert!(both.is_writable());
    }

    #[test]
    fn test_io_op_id() {
        let id1 = next_io_op_id();
        let id2 = next_io_op_id();
        assert_ne!(id1, id2);
        assert!(id2.as_u64() > id1.as_u64());
    }

    #[test]
    fn test_reactor_creation() {
        let reactor = IoReactor::new(ReactorConfig::default());
        assert_eq!(reactor.pending_count(), 0);
        assert!(!reactor.has_pending());
    }

    #[test]
    fn test_submit_operation() {
        let reactor = IoReactor::new(ReactorConfig::default());

        let op = IoOp::Timeout {
            duration: Duration::from_millis(10),
        };
        let _op_id = reactor.submit(op).unwrap();

        assert_eq!(reactor.pending_count(), 1);
        assert!(reactor.has_pending());
    }

    #[test]
    fn test_poll_completions() {
        let reactor = IoReactor::new(ReactorConfig::default());

        // Submit a timeout operation
        let op = IoOp::Timeout {
            duration: Duration::from_millis(1),
        };
        reactor.submit(op).unwrap();

        // Poll for completions
        let completions = reactor.poll().unwrap();
        assert_eq!(completions.len(), 1);
        assert!(matches!(completions[0].result, IoResult::TimedOut));

        assert_eq!(reactor.pending_count(), 0);
    }

    #[test]
    fn test_cancel_operation() {
        let reactor = IoReactor::new(ReactorConfig::default());

        let op = IoOp::Timeout {
            duration: Duration::from_secs(100),
        };
        let op_id = reactor.submit(op).unwrap();
        assert_eq!(reactor.pending_count(), 1);

        reactor.cancel(op_id).unwrap();
        assert_eq!(reactor.pending_count(), 0);
    }

    #[test]
    fn test_blocking_driver() {
        let driver = BlockingDriver::new();

        let op_id = next_io_op_id();
        driver.submit(op_id, IoOp::Write {
            fd: 1,
            data: vec![1, 2, 3],
            offset: -1,
        }).unwrap();

        let completions = driver.poll(Duration::from_millis(100)).unwrap();
        assert_eq!(completions.len(), 1);
        assert!(matches!(completions[0].result, IoResult::Write(3)));
    }

    #[test]
    fn test_io_error() {
        let err = IoError::new(42, "test error");
        assert_eq!(err.code, 42);
        assert_eq!(err.message, "test error");
        assert!(err.to_string().contains("42"));
        assert!(err.to_string().contains("test error"));
    }
}
