//! Run in a separate test executable because the network guard patches ws2_32 process-wide.
//! Remote destinations are TEST-NET-1 (192.0.2.0/24) and an `.invalid` name, so a broken
//! hook cannot reach a real host.
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

const WSAENETUNREACH: i32 = 10051;
const WSAHOST_NOT_FOUND: i32 = 11001;

#[repr(C)]
struct AddrInfoA {
    flags: i32,
    family: i32,
    socktype: i32,
    protocol: i32,
    addrlen: usize,
    canonname: *mut u8,
    addr: *mut u8,
    next: *mut AddrInfoA,
}

#[link(name = "ws2_32")]
extern "system" {
    fn getaddrinfo(node: *const u8, service: *const u8, hints: *const AddrInfoA, res: *mut *mut AddrInfoA) -> i32;
    fn freeaddrinfo(res: *mut AddrInfoA);
}

#[test]
fn deny_mode_passes_local_traffic_only() {
    std::env::remove_var("TOMBOX_ALLOW_NETWORK");
    std::env::remove_var("TOMBOX_NET_REDIRECT");
    // Load and initialise Winsock before the guard patches it.
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    compat_common::net_guard::ensure();

    // Loopback datagrams pass in both directions.
    udp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    udp.send_to(b"ping", udp.local_addr().unwrap()).unwrap();
    let mut buf = [0u8; 8];
    let (n, _) = udp.recv_from(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"ping");

    // Any other destination is refused before it reaches the network.
    let err = udp.send_to(b"x", "192.0.2.1:9").unwrap_err();
    assert_eq!(err.raw_os_error(), Some(WSAENETUNREACH), "{err}");
    let err = TcpStream::connect_timeout(&"192.0.2.1:9".parse().unwrap(), Duration::from_secs(5)).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(WSAENETUNREACH), "{err}");

    // A loopback TCP connection is accepted.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    assert_eq!(server.peer_addr().unwrap(), client.local_addr().unwrap());

    // Names that need no name server resolve; any other name is refused.
    assert!(("localhost", 0).to_socket_addrs().unwrap().next().is_some());
    assert!("192.0.2.1:9".to_socket_addrs().unwrap().next().is_some());
    let err = ("tuxbox-net-guard-test.invalid", 80).to_socket_addrs().unwrap_err();
    assert_eq!(err.raw_os_error(), Some(WSAHOST_NOT_FOUND), "{err}");
    assert_ne!(err.kind(), ErrorKind::TimedOut);

    // getaddrinfo(NULL, port, AI_PASSIVE): the wildcard address for binding a local socket.
    let hints = AddrInfoA {
        flags: 1, // AI_PASSIVE
        family: 2, // AF_INET
        socktype: 2, // SOCK_DGRAM
        protocol: 0,
        addrlen: 0,
        canonname: std::ptr::null_mut(),
        addr: std::ptr::null_mut(),
        next: std::ptr::null_mut(),
    };
    let mut res = std::ptr::null_mut();
    // SAFETY: valid hints and output pointer; the result is freed below.
    unsafe {
        assert_eq!(getaddrinfo(std::ptr::null(), b"19132\0".as_ptr(), &hints, &mut res), 0);
        assert!(!res.is_null());
        let sa = (*res).addr;
        assert_eq!(u16::from_ne_bytes([*sa, *sa.add(1)]), 2);
        assert_eq!(std::slice::from_raw_parts(sa.add(4), 4), [0, 0, 0, 0], "wildcard address");
        freeaddrinfo(res);
    }
}
