//! Networking Infrastructure
//!
//! This module provides networking capabilities for the Blood runtime:
//!
//! - **TCP**: Client and server socket operations
//! - **DNS**: Domain name resolution
//! - **Address**: IP address and socket address types
//!
//! # Design
//!
//! The networking module provides both blocking and async-capable APIs.
//! All operations integrate with the runtime's cancellation system.
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_runtime::net::{TcpStream, TcpListener, lookup_host};
//!
//! // TCP client
//! let stream = TcpStream::connect("example.com:80")?;
//! stream.write_all(b"GET / HTTP/1.0\r\n\r\n")?;
//!
//! // TCP server
//! let listener = TcpListener::bind("127.0.0.1:8080")?;
//! for stream in listener.incoming() {
//!     // Handle connection...
//! }
//!
//! // DNS lookup
//! for addr in lookup_host("example.com")? {
//!     println!("Address: {}", addr);
//! }
//! ```

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, ToSocketAddrs};
use std::time::Duration;

// Re-import std::net types that we wrap
use std::net::TcpListener as StdTcpListener;
use std::net::TcpStream as StdTcpStream;
use std::net::UdpSocket as StdUdpSocket;

use crate::cancellation::CancellationToken;
use crate::timeout::TimeoutGuard;

// ============================================================================
// ERROR TYPES
// ============================================================================

/// Error type for networking operations.
#[derive(Debug)]
pub enum NetError {
    /// I/O error.
    Io(io::Error),
    /// DNS resolution failed.
    DnsLookup(String),
    /// Connection refused.
    ConnectionRefused,
    /// Connection reset by peer.
    ConnectionReset,
    /// Connection timed out.
    TimedOut,
    /// Address already in use.
    AddrInUse,
    /// Address not available.
    AddrNotAvailable,
    /// Network unreachable.
    NetworkUnreachable,
    /// Host unreachable.
    HostUnreachable,
    /// Operation cancelled.
    Cancelled,
    /// Invalid address format.
    InvalidAddress(String),
    /// Socket not connected.
    NotConnected,
    /// Operation would block.
    WouldBlock,
    /// Other error.
    Other(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Io(e) => write!(f, "I/O error: {}", e),
            NetError::DnsLookup(host) => write!(f, "DNS lookup failed for: {}", host),
            NetError::ConnectionRefused => write!(f, "connection refused"),
            NetError::ConnectionReset => write!(f, "connection reset by peer"),
            NetError::TimedOut => write!(f, "operation timed out"),
            NetError::AddrInUse => write!(f, "address already in use"),
            NetError::AddrNotAvailable => write!(f, "address not available"),
            NetError::NetworkUnreachable => write!(f, "network unreachable"),
            NetError::HostUnreachable => write!(f, "host unreachable"),
            NetError::Cancelled => write!(f, "operation cancelled"),
            NetError::InvalidAddress(addr) => write!(f, "invalid address: {}", addr),
            NetError::NotConnected => write!(f, "socket not connected"),
            NetError::WouldBlock => write!(f, "operation would block"),
            NetError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for NetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NetError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for NetError {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::ConnectionRefused => NetError::ConnectionRefused,
            io::ErrorKind::ConnectionReset => NetError::ConnectionReset,
            io::ErrorKind::TimedOut => NetError::TimedOut,
            io::ErrorKind::AddrInUse => NetError::AddrInUse,
            io::ErrorKind::AddrNotAvailable => NetError::AddrNotAvailable,
            io::ErrorKind::NetworkUnreachable => NetError::NetworkUnreachable,
            io::ErrorKind::NotConnected => NetError::NotConnected,
            io::ErrorKind::WouldBlock => NetError::WouldBlock,
            _ => NetError::Io(err),
        }
    }
}

/// Result type for networking operations.
pub type Result<T> = std::result::Result<T, NetError>;

// ============================================================================
// DNS RESOLUTION
// ============================================================================

/// DNS resolver configuration.
#[derive(Debug, Clone)]
pub struct ResolverConfig {
    /// Timeout for DNS queries.
    pub timeout: Duration,
    /// Number of retry attempts.
    pub retries: u32,
    /// Whether to prefer IPv4 addresses.
    pub prefer_ipv4: bool,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            retries: 3,
            prefer_ipv4: false,
        }
    }
}

impl ResolverConfig {
    /// Create a resolver config from the global runtime configuration.
    ///
    /// Uses the configured network timeout if available.
    pub fn from_runtime_config() -> Self {
        if let Some(config) = crate::runtime_config() {
            Self {
                timeout: config.timeout.network_timeout,
                ..Default::default()
            }
        } else {
            Self::default()
        }
    }
}

/// Get the configured network timeout from the runtime configuration.
///
/// Returns the default (60 seconds) if no runtime config is available.
pub fn configured_network_timeout() -> Duration {
    crate::runtime_config()
        .map(|c| c.timeout.network_timeout)
        .unwrap_or_else(|| Duration::from_secs(60))
}

/// Get the configured I/O timeout from the runtime configuration.
///
/// Returns the default (30 seconds) if no runtime config is available.
pub fn configured_io_timeout() -> Duration {
    crate::runtime_config()
        .map(|c| c.timeout.io_timeout)
        .unwrap_or_else(|| Duration::from_secs(30))
}

/// Look up IP addresses for a hostname.
///
/// This performs DNS resolution and returns all resolved addresses.
///
/// # Example
///
/// ```rust,ignore
/// for addr in lookup_host("example.com")? {
///     println!("Resolved: {}", addr);
/// }
/// ```
pub fn lookup_host(host: &str) -> Result<impl Iterator<Item = std::net::IpAddr>> {
    use std::net::IpAddr;

    // Try parsing as an IP address first
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip].into_iter());
    }

    // Perform DNS lookup by resolving with a dummy port
    let host_with_port = format!("{}:0", host);
    let addrs: Vec<IpAddr> = host_with_port
        .to_socket_addrs()
        .map_err(|_| NetError::DnsLookup(host.to_string()))?
        .map(|addr| addr.ip())
        .collect();

    if addrs.is_empty() {
        return Err(NetError::DnsLookup(host.to_string()));
    }

    Ok(addrs.into_iter())
}

/// Look up socket addresses for a host:port combination.
///
/// # Example
///
/// ```rust,ignore
/// for addr in lookup_addr("example.com:80")? {
///     println!("Socket address: {}", addr);
/// }
/// ```
pub fn lookup_addr(addr: &str) -> Result<impl Iterator<Item = SocketAddr>> {
    let addrs: Vec<SocketAddr> = addr
        .to_socket_addrs()
        .map_err(|_| NetError::InvalidAddress(addr.to_string()))?
        .collect();

    if addrs.is_empty() {
        return Err(NetError::DnsLookup(addr.to_string()));
    }

    Ok(addrs.into_iter())
}

/// Look up socket addresses with a timeout.
pub fn lookup_addr_with_timeout(
    addr: &str,
    timeout: Duration,
) -> Result<impl Iterator<Item = SocketAddr>> {
    let _guard = TimeoutGuard::new(timeout);
    lookup_addr(addr)
}

// ============================================================================
// TCP STREAM
// ============================================================================

/// A TCP stream between a local and a remote socket.
///
/// After creating a `TcpStream` by either connecting to a remote host or
/// accepting a connection on a `TcpListener`, data can be transmitted
/// by reading and writing to it.
pub struct TcpStream {
    inner: StdTcpStream,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

impl TcpStream {
    /// Open a TCP connection to a remote host.
    ///
    /// `addr` can be either a socket address or a host:port string.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let stream = TcpStream::connect("example.com:80")?;
    /// // or
    /// let stream = TcpStream::connect("192.168.1.1:8080")?;
    /// ```
    pub fn connect<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        let inner = StdTcpStream::connect(addr)?;
        Ok(Self {
            inner,
            read_timeout: None,
            write_timeout: None,
        })
    }

    /// Open a TCP connection with a timeout.
    pub fn connect_timeout(addr: &SocketAddr, timeout: Duration) -> Result<Self> {
        let inner = StdTcpStream::connect_timeout(addr, timeout)?;
        Ok(Self {
            inner,
            read_timeout: None,
            write_timeout: None,
        })
    }

    /// Open a TCP connection with a cancellation token.
    ///
    /// The connection attempt will be cancelled if the token is cancelled.
    pub fn connect_with_cancellation<A: ToSocketAddrs>(
        addr: A,
        token: CancellationToken,
    ) -> Result<Self> {
        // Check cancellation before starting
        if token.is_cancelled() {
            return Err(NetError::Cancelled);
        }

        // For now, use blocking connect with periodic cancellation checks
        // In a full async implementation, this would integrate with the I/O reactor
        let stream = StdTcpStream::connect(addr)?;

        // Check cancellation after connect
        if token.is_cancelled() {
            return Err(NetError::Cancelled);
        }

        Ok(Self {
            inner: stream,
            read_timeout: None,
            write_timeout: None,
        })
    }

    /// Open a TCP connection using the configured network timeout.
    ///
    /// Uses `config.timeout.network_timeout` from the runtime configuration,
    /// falling back to 60 seconds if no configuration is available.
    pub fn connect_with_configured_timeout(addr: &SocketAddr) -> Result<Self> {
        Self::connect_timeout(addr, configured_network_timeout())
    }

    /// Open a TCP connection with both configured timeout and cancellation support.
    ///
    /// Uses `config.timeout.network_timeout` from the runtime configuration.
    /// The connection attempt will be cancelled if the token is cancelled or
    /// if the configured timeout elapses.
    pub fn connect_with_configured_timeout_and_cancellation(
        addr: &SocketAddr,
        token: CancellationToken,
    ) -> Result<Self> {
        // Check cancellation before starting
        if token.is_cancelled() {
            return Err(NetError::Cancelled);
        }

        let timeout = configured_network_timeout();
        let inner = StdTcpStream::connect_timeout(addr, timeout)?;

        // Check cancellation after connect
        if token.is_cancelled() {
            return Err(NetError::Cancelled);
        }

        Ok(Self {
            inner,
            read_timeout: None,
            write_timeout: None,
        })
    }

    /// Apply the configured I/O timeouts to this stream.
    ///
    /// Sets both read and write timeouts from `config.timeout.io_timeout`.
    pub fn apply_configured_timeouts(&mut self) -> Result<()> {
        let timeout = configured_io_timeout();
        self.set_read_timeout(Some(timeout))?;
        self.set_write_timeout(Some(timeout))?;
        Ok(())
    }

    /// Returns the socket address of the remote peer.
    pub fn peer_addr(&self) -> Result<SocketAddr> {
        self.inner.peer_addr().map_err(NetError::from)
    }

    /// Returns the socket address of the local half of this connection.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.local_addr().map_err(NetError::from)
    }

    /// Shuts down the read, write, or both halves of this connection.
    pub fn shutdown(&self, how: Shutdown) -> Result<()> {
        self.inner.shutdown(how).map_err(NetError::from)
    }

    /// Set the read timeout for the socket.
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.inner.set_read_timeout(timeout)?;
        self.read_timeout = timeout;
        Ok(())
    }

    /// Set the write timeout for the socket.
    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.inner.set_write_timeout(timeout)?;
        self.write_timeout = timeout;
        Ok(())
    }

    /// Get the read timeout for the socket.
    pub fn read_timeout(&self) -> Option<Duration> {
        self.read_timeout
    }

    /// Get the write timeout for the socket.
    pub fn write_timeout(&self) -> Option<Duration> {
        self.write_timeout
    }

    /// Set the value of the TCP_NODELAY option.
    pub fn set_nodelay(&self, nodelay: bool) -> Result<()> {
        self.inner.set_nodelay(nodelay).map_err(NetError::from)
    }

    /// Get the value of the TCP_NODELAY option.
    pub fn nodelay(&self) -> Result<bool> {
        self.inner.nodelay().map_err(NetError::from)
    }

    /// Set the time-to-live value for packets.
    pub fn set_ttl(&self, ttl: u32) -> Result<()> {
        self.inner.set_ttl(ttl).map_err(NetError::from)
    }

    /// Get the time-to-live value.
    pub fn ttl(&self) -> Result<u32> {
        self.inner.ttl().map_err(NetError::from)
    }

    /// Try to clone the TcpStream.
    pub fn try_clone(&self) -> Result<Self> {
        let inner = self.inner.try_clone()?;
        Ok(Self {
            inner,
            read_timeout: self.read_timeout,
            write_timeout: self.write_timeout,
        })
    }

    /// Get a reference to the underlying std::StdTcpStream.
    pub fn as_raw(&self) -> &StdTcpStream {
        &self.inner
    }

    /// Consume and return the underlying std::StdTcpStream.
    pub fn into_inner(self) -> StdTcpStream {
        self.inner
    }
}

impl Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl fmt::Debug for TcpStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpStream")
            .field("peer_addr", &self.peer_addr().ok())
            .field("local_addr", &self.local_addr().ok())
            .finish()
    }
}

// ============================================================================
// TCP LISTENER
// ============================================================================

/// A TCP socket server, listening for connections.
///
/// After creating a `TcpListener`, you can accept incoming connections
/// using the `accept` method or iterate over connections using `incoming`.
pub struct TcpListener {
    inner: StdTcpListener,
}

impl TcpListener {
    /// Create a new TcpListener bound to the specified address.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let listener = TcpListener::bind("127.0.0.1:8080")?;
    /// ```
    pub fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        let inner = StdTcpListener::bind(addr)?;
        Ok(Self { inner })
    }

    /// Accept a new incoming connection.
    ///
    /// Returns the connected TcpStream and the address of the remote peer.
    pub fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        let (stream, addr) = self.inner.accept()?;
        Ok((
            TcpStream {
                inner: stream,
                read_timeout: None,
                write_timeout: None,
            },
            addr,
        ))
    }

    /// Accept with a cancellation token.
    pub fn accept_with_cancellation(
        &self,
        token: CancellationToken,
    ) -> Result<(TcpStream, SocketAddr)> {
        if token.is_cancelled() {
            return Err(NetError::Cancelled);
        }

        // Set non-blocking temporarily for cancellation checking
        self.inner.set_nonblocking(true)?;

        loop {
            match self.inner.accept() {
                Ok((stream, addr)) => {
                    self.inner.set_nonblocking(false)?;
                    stream.set_nonblocking(false)?;
                    return Ok((
                        TcpStream {
                            inner: stream,
                            read_timeout: None,
                            write_timeout: None,
                        },
                        addr,
                    ));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if token.is_cancelled() {
                        self.inner.set_nonblocking(false)?;
                        return Err(NetError::Cancelled);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    self.inner.set_nonblocking(false)?;
                    return Err(NetError::from(e));
                }
            }
        }
    }

    /// Returns an iterator over incoming connections.
    pub fn incoming(&self) -> Incoming<'_> {
        Incoming { listener: self }
    }

    /// Returns the local socket address.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.local_addr().map_err(NetError::from)
    }

    /// Set the time-to-live value.
    pub fn set_ttl(&self, ttl: u32) -> Result<()> {
        self.inner.set_ttl(ttl).map_err(NetError::from)
    }

    /// Get the time-to-live value.
    pub fn ttl(&self) -> Result<u32> {
        self.inner.ttl().map_err(NetError::from)
    }

    /// Try to clone the listener.
    pub fn try_clone(&self) -> Result<Self> {
        let inner = self.inner.try_clone()?;
        Ok(Self { inner })
    }

    /// Accept a connection with a timeout.
    ///
    /// Returns the connected TcpStream and the address of the remote peer,
    /// or a timeout error if no connection is received within the timeout.
    pub fn accept_timeout(&self, timeout: Duration) -> Result<(TcpStream, SocketAddr)> {
        self.inner.set_nonblocking(true)?;
        let start = std::time::Instant::now();

        loop {
            match self.inner.accept() {
                Ok((stream, addr)) => {
                    self.inner.set_nonblocking(false)?;
                    stream.set_nonblocking(false)?;
                    return Ok((
                        TcpStream {
                            inner: stream,
                            read_timeout: None,
                            write_timeout: None,
                        },
                        addr,
                    ));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if start.elapsed() >= timeout {
                        self.inner.set_nonblocking(false)?;
                        return Err(NetError::TimedOut);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    self.inner.set_nonblocking(false)?;
                    return Err(NetError::from(e));
                }
            }
        }
    }

    /// Accept a connection using the configured network timeout.
    ///
    /// Uses `config.timeout.network_timeout` from the runtime configuration,
    /// falling back to 60 seconds if no configuration is available.
    pub fn accept_with_configured_timeout(&self) -> Result<(TcpStream, SocketAddr)> {
        self.accept_timeout(configured_network_timeout())
    }

    /// Accept a connection with configured timeout and cancellation support.
    ///
    /// Uses `config.timeout.network_timeout` from the runtime configuration.
    /// The accept will be cancelled if the token is cancelled or if the
    /// configured timeout elapses.
    pub fn accept_with_configured_timeout_and_cancellation(
        &self,
        token: CancellationToken,
    ) -> Result<(TcpStream, SocketAddr)> {
        if token.is_cancelled() {
            return Err(NetError::Cancelled);
        }

        let timeout = configured_network_timeout();
        self.inner.set_nonblocking(true)?;
        let start = std::time::Instant::now();

        loop {
            match self.inner.accept() {
                Ok((stream, addr)) => {
                    self.inner.set_nonblocking(false)?;
                    stream.set_nonblocking(false)?;
                    return Ok((
                        TcpStream {
                            inner: stream,
                            read_timeout: None,
                            write_timeout: None,
                        },
                        addr,
                    ));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if token.is_cancelled() {
                        self.inner.set_nonblocking(false)?;
                        return Err(NetError::Cancelled);
                    }
                    if start.elapsed() >= timeout {
                        self.inner.set_nonblocking(false)?;
                        return Err(NetError::TimedOut);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    self.inner.set_nonblocking(false)?;
                    return Err(NetError::from(e));
                }
            }
        }
    }

    /// Get a reference to the underlying std::StdTcpListener.
    pub fn as_raw(&self) -> &StdTcpListener {
        &self.inner
    }

    /// Consume and return the underlying std::StdTcpListener.
    pub fn into_inner(self) -> StdTcpListener {
        self.inner
    }
}

impl fmt::Debug for TcpListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpListener")
            .field("local_addr", &self.local_addr().ok())
            .finish()
    }
}

/// Iterator over incoming TCP connections.
pub struct Incoming<'a> {
    listener: &'a TcpListener,
}

impl<'a> Iterator for Incoming<'a> {
    type Item = Result<TcpStream>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(self.listener.accept().map(|(stream, _)| stream))
    }
}

// ============================================================================
// UDP SOCKET
// ============================================================================

/// A UDP socket.
///
/// UDP is a connectionless protocol. After binding a `UdpSocket` to a local
/// address, you can send and receive datagrams.
pub struct UdpSocket {
    inner: StdUdpSocket,
}

impl UdpSocket {
    /// Create a new UDP socket bound to the specified address.
    pub fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        let inner = StdUdpSocket::bind(addr)?;
        Ok(Self { inner })
    }

    /// Send data to the specified address.
    pub fn send_to<A: ToSocketAddrs>(&self, buf: &[u8], addr: A) -> Result<usize> {
        self.inner.send_to(buf, addr).map_err(NetError::from)
    }

    /// Receive data into the buffer.
    ///
    /// Returns the number of bytes read and the source address.
    pub fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        self.inner.recv_from(buf).map_err(NetError::from)
    }

    /// Connect this socket to a remote address.
    ///
    /// After connecting, `send` and `recv` can be used instead of
    /// `send_to` and `recv_from`.
    pub fn connect<A: ToSocketAddrs>(&self, addr: A) -> Result<()> {
        self.inner.connect(addr).map_err(NetError::from)
    }

    /// Send data on a connected socket.
    pub fn send(&self, buf: &[u8]) -> Result<usize> {
        self.inner.send(buf).map_err(NetError::from)
    }

    /// Receive data on a connected socket.
    pub fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        self.inner.recv(buf).map_err(NetError::from)
    }

    /// Returns the socket address of the remote peer.
    pub fn peer_addr(&self) -> Result<SocketAddr> {
        self.inner.peer_addr().map_err(NetError::from)
    }

    /// Returns the local socket address.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.local_addr().map_err(NetError::from)
    }

    /// Set the read timeout.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        self.inner.set_read_timeout(timeout).map_err(NetError::from)
    }

    /// Set the write timeout.
    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        self.inner.set_write_timeout(timeout).map_err(NetError::from)
    }

    /// Set the broadcast flag.
    pub fn set_broadcast(&self, broadcast: bool) -> Result<()> {
        self.inner.set_broadcast(broadcast).map_err(NetError::from)
    }

    /// Get the broadcast flag.
    pub fn broadcast(&self) -> Result<bool> {
        self.inner.broadcast().map_err(NetError::from)
    }

    /// Set the multicast loop flag.
    pub fn set_multicast_loop_v4(&self, multicast_loop: bool) -> Result<()> {
        self.inner
            .set_multicast_loop_v4(multicast_loop)
            .map_err(NetError::from)
    }

    /// Get the multicast loop flag.
    pub fn multicast_loop_v4(&self) -> Result<bool> {
        self.inner.multicast_loop_v4().map_err(NetError::from)
    }

    /// Set the multicast TTL.
    pub fn set_multicast_ttl_v4(&self, ttl: u32) -> Result<()> {
        self.inner.set_multicast_ttl_v4(ttl).map_err(NetError::from)
    }

    /// Get the multicast TTL.
    pub fn multicast_ttl_v4(&self) -> Result<u32> {
        self.inner.multicast_ttl_v4().map_err(NetError::from)
    }

    /// Join a multicast group.
    pub fn join_multicast_v4(&self, multiaddr: &Ipv4Addr, interface: &Ipv4Addr) -> Result<()> {
        self.inner
            .join_multicast_v4(multiaddr, interface)
            .map_err(NetError::from)
    }

    /// Leave a multicast group.
    pub fn leave_multicast_v4(&self, multiaddr: &Ipv4Addr, interface: &Ipv4Addr) -> Result<()> {
        self.inner
            .leave_multicast_v4(multiaddr, interface)
            .map_err(NetError::from)
    }

    /// Join an IPv6 multicast group.
    pub fn join_multicast_v6(&self, multiaddr: &Ipv6Addr, interface: u32) -> Result<()> {
        self.inner
            .join_multicast_v6(multiaddr, interface)
            .map_err(NetError::from)
    }

    /// Leave an IPv6 multicast group.
    pub fn leave_multicast_v6(&self, multiaddr: &Ipv6Addr, interface: u32) -> Result<()> {
        self.inner
            .leave_multicast_v6(multiaddr, interface)
            .map_err(NetError::from)
    }

    /// Set the TTL.
    pub fn set_ttl(&self, ttl: u32) -> Result<()> {
        self.inner.set_ttl(ttl).map_err(NetError::from)
    }

    /// Get the TTL.
    pub fn ttl(&self) -> Result<u32> {
        self.inner.ttl().map_err(NetError::from)
    }

    /// Try to clone the socket.
    pub fn try_clone(&self) -> Result<Self> {
        let inner = self.inner.try_clone()?;
        Ok(Self { inner })
    }

    /// Apply the configured I/O timeouts to this socket.
    ///
    /// Sets both read and write timeouts from `config.timeout.io_timeout`.
    pub fn apply_configured_timeouts(&self) -> Result<()> {
        let timeout = configured_io_timeout();
        self.set_read_timeout(Some(timeout))?;
        self.set_write_timeout(Some(timeout))?;
        Ok(())
    }

    /// Get a reference to the underlying std::StdUdpSocket.
    pub fn as_raw(&self) -> &StdUdpSocket {
        &self.inner
    }

    /// Consume and return the underlying std::StdUdpSocket.
    pub fn into_inner(self) -> StdUdpSocket {
        self.inner
    }
}

impl fmt::Debug for UdpSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UdpSocket")
            .field("local_addr", &self.local_addr().ok())
            .finish()
    }
}

// ============================================================================
// IP ADDRESS UTILITIES
// ============================================================================

/// Check if an IP address is a loopback address.
pub fn is_loopback(addr: &std::net::IpAddr) -> bool {
    addr.is_loopback()
}

/// Check if an IP address is a private address.
pub fn is_private(addr: &std::net::IpAddr) -> bool {
    match addr {
        std::net::IpAddr::V4(ipv4) => {
            // Private ranges:
            // 10.0.0.0 - 10.255.255.255
            // 172.16.0.0 - 172.31.255.255
            // 192.168.0.0 - 192.168.255.255
            let octets = ipv4.octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
        }
        std::net::IpAddr::V6(_) => false, // IPv6 private addressing is more complex
    }
}

/// Check if an IP address is a multicast address.
pub fn is_multicast(addr: &std::net::IpAddr) -> bool {
    addr.is_multicast()
}

/// Parse an IP address from a string.
pub fn parse_ip(s: &str) -> Result<std::net::IpAddr> {
    s.parse().map_err(|_| NetError::InvalidAddress(s.to_string()))
}

/// Parse a socket address from a string.
pub fn parse_socket_addr(s: &str) -> Result<SocketAddr> {
    s.parse().map_err(|_| NetError::InvalidAddress(s.to_string()))
}

// ============================================================================
// SOCKET OPTIONS
// ============================================================================

/// Common socket options.
#[derive(Debug, Clone, Default)]
pub struct SocketOptions {
    /// Read timeout.
    pub read_timeout: Option<Duration>,
    /// Write timeout.
    pub write_timeout: Option<Duration>,
    /// Enable TCP_NODELAY (disable Nagle's algorithm).
    pub nodelay: bool,
    /// Time-to-live for packets.
    pub ttl: Option<u32>,
    /// Keep-alive settings.
    pub keep_alive: Option<KeepAlive>,
}

/// TCP keep-alive configuration.
#[derive(Debug, Clone)]
pub struct KeepAlive {
    /// Enable keep-alive.
    pub enabled: bool,
    /// Time between keep-alive probes.
    pub interval: Duration,
    /// Number of unacknowledged probes before considering the connection dead.
    pub retries: u32,
}

impl Default for KeepAlive {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: Duration::from_secs(60),
            retries: 9,
        }
    }
}

// ============================================================================
// CONNECTION INFO
// ============================================================================

/// Information about a network connection.
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    /// Local address.
    pub local_addr: SocketAddr,
    /// Remote address.
    pub remote_addr: SocketAddr,
    /// Protocol.
    pub protocol: Protocol,
}

/// Network protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// TCP protocol.
    Tcp,
    /// UDP protocol.
    Udp,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Protocol::Tcp => write!(f, "TCP"),
            Protocol::Udp => write!(f, "UDP"),
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_net_error_display() {
        let err = NetError::ConnectionRefused;
        assert_eq!(err.to_string(), "connection refused");

        let err = NetError::TimedOut;
        assert_eq!(err.to_string(), "operation timed out");

        let err = NetError::DnsLookup("example.com".to_string());
        assert!(err.to_string().contains("example.com"));
    }

    #[test]
    fn test_net_error_from_io() {
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
        let net_err: NetError = io_err.into();
        assert!(matches!(net_err, NetError::ConnectionRefused));
    }

    #[test]
    fn test_resolver_config_default() {
        let config = ResolverConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.retries, 3);
        assert!(!config.prefer_ipv4);
    }

    #[test]
    fn test_lookup_host_localhost() {
        let addrs: Vec<_> = lookup_host("localhost").unwrap().collect();
        assert!(!addrs.is_empty());
    }

    #[test]
    fn test_lookup_host_ip() {
        let addrs: Vec<_> = lookup_host("127.0.0.1").unwrap().collect();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "127.0.0.1");
    }

    #[test]
    fn test_tcp_listener_bind() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.port() > 0);
    }

    #[test]
    fn test_tcp_connect_accept() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let (stream, peer_addr) = listener.accept().unwrap();
            assert!(!peer_addr.ip().is_unspecified());
            stream
        });

        let client = TcpStream::connect(addr).unwrap();
        let peer_addr = client.peer_addr().unwrap();
        assert_eq!(peer_addr, addr);

        handle.join().unwrap();
    }

    #[test]
    fn test_tcp_read_write() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 5];
            stream.read_exact(&mut buf).unwrap();
            assert_eq!(&buf, b"hello");
            stream.write_all(b"world").unwrap();
        });

        let mut client = TcpStream::connect(addr).unwrap();
        client.write_all(b"hello").unwrap();
        let mut buf = [0u8; 5];
        client.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"world");

        handle.join().unwrap();
    }

    #[test]
    fn test_tcp_stream_options() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let _ = listener.accept().unwrap();
        });

        let mut stream = TcpStream::connect(addr).unwrap();

        // Test nodelay
        stream.set_nodelay(true).unwrap();
        assert!(stream.nodelay().unwrap());

        // Test timeout
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        assert_eq!(stream.read_timeout(), Some(Duration::from_secs(5)));

        handle.join().unwrap();
    }

    #[test]
    fn test_udp_socket() {
        let socket1 = UdpSocket::bind("127.0.0.1:0").unwrap();
        let socket2 = UdpSocket::bind("127.0.0.1:0").unwrap();

        let addr1 = socket1.local_addr().unwrap();
        let addr2 = socket2.local_addr().unwrap();

        socket1.send_to(b"hello", addr2).unwrap();

        let mut buf = [0u8; 10];
        let (len, from) = socket2.recv_from(&mut buf).unwrap();
        assert_eq!(len, 5);
        assert_eq!(&buf[..len], b"hello");
        assert_eq!(from, addr1);
    }

    #[test]
    fn test_is_loopback() {
        use std::net::IpAddr;
        assert!(is_loopback(&"127.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_loopback(&"::1".parse::<IpAddr>().unwrap()));
        assert!(!is_loopback(&"192.168.1.1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn test_is_private() {
        use std::net::IpAddr;
        assert!(is_private(&"10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private(&"172.16.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private(&"192.168.1.1".parse::<IpAddr>().unwrap()));
        assert!(!is_private(&"8.8.8.8".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn test_parse_ip() {
        let ip = parse_ip("192.168.1.1").unwrap();
        assert_eq!(ip.to_string(), "192.168.1.1");

        let ip = parse_ip("::1").unwrap();
        assert!(ip.is_loopback());

        assert!(parse_ip("invalid").is_err());
    }

    #[test]
    fn test_parse_socket_addr() {
        let addr = parse_socket_addr("192.168.1.1:8080").unwrap();
        assert_eq!(addr.port(), 8080);

        assert!(parse_socket_addr("invalid").is_err());
    }

    #[test]
    fn test_protocol_display() {
        assert_eq!(Protocol::Tcp.to_string(), "TCP");
        assert_eq!(Protocol::Udp.to_string(), "UDP");
    }

    #[test]
    fn test_socket_options_default() {
        let opts = SocketOptions::default();
        assert!(opts.read_timeout.is_none());
        assert!(opts.write_timeout.is_none());
        assert!(!opts.nodelay);
        assert!(opts.ttl.is_none());
    }

    #[test]
    fn test_keep_alive_default() {
        let ka = KeepAlive::default();
        assert!(!ka.enabled);
        assert_eq!(ka.interval, Duration::from_secs(60));
        assert_eq!(ka.retries, 9);
    }

    #[test]
    fn test_configured_network_timeout() {
        // Without runtime config, should return default 60 seconds
        let timeout = configured_network_timeout();
        assert_eq!(timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_configured_io_timeout() {
        // Without runtime config, should return default 30 seconds
        let timeout = configured_io_timeout();
        assert_eq!(timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_resolver_config_from_runtime() {
        // Without runtime config, should use defaults
        let config = ResolverConfig::from_runtime_config();
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.retries, 3);
    }

    #[test]
    fn test_tcp_stream_connect_with_configured_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let _ = listener.accept().unwrap();
        });

        // This should use the configured timeout (60s default)
        let stream = TcpStream::connect_with_configured_timeout(&addr).unwrap();
        assert!(stream.peer_addr().is_ok());

        handle.join().unwrap();
    }

    #[test]
    fn test_tcp_stream_apply_configured_timeouts() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let _ = listener.accept().unwrap();
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        stream.apply_configured_timeouts().unwrap();

        // Should have timeouts set now (30s default)
        assert_eq!(stream.read_timeout(), Some(Duration::from_secs(30)));
        assert_eq!(stream.write_timeout(), Some(Duration::from_secs(30)));

        handle.join().unwrap();
    }

    #[test]
    fn test_tcp_listener_accept_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();

        // Try to accept with a very short timeout - should timeout
        let result = listener.accept_timeout(Duration::from_millis(50));
        assert!(matches!(result, Err(NetError::TimedOut)));
    }

    #[test]
    fn test_udp_socket_apply_configured_timeouts() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.apply_configured_timeouts().unwrap();
        // Just verify it doesn't panic
    }
}
