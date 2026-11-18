//! Reaching another node: TCP connect times and open ports.
//!
//! A TCP connect takes one round trip (SYN, SYN-ACK), so its duration is a fair measure of the
//! distance to the server without ICMP, which clouds often block. UDP has no handshake: a UDP port
//! cannot be shown open from one side.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::time::timeout;

/// How long one connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Every address `host:port` resolves to, such as an IPv6 and an IPv4 address.
async fn addresses(host: &str, port: u16) -> Vec<SocketAddr> {
    tokio::net::lookup_host((host, port))
        .await
        .map_or_else(|_e| Vec::new(), Iterator::collect)
}

/// The first of `addrs` that a TCP connection reaches within [`CONNECT_TIMEOUT`].
async fn first_open(addrs: &[SocketAddr]) -> Option<SocketAddr> {
    for &addr in addrs {
        if connect(addr).await {
            return Some(addr);
        }
    }
    None
}

/// Whether a TCP connection to `addr` succeeds within [`CONNECT_TIMEOUT`].
async fn connect(addr: SocketAddr) -> bool {
    timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .is_ok_and(|r| r.is_ok())
}

/// The durations, in microseconds, of `samples` TCP connections to `host:port`, at the first of
/// its addresses that answers; failed ones are left out, so an empty list means no address
/// answered.
#[must_use]
pub async fn connect_times(host: &str, port: u16, samples: usize) -> Vec<u64> {
    let Some(addr) = first_open(&addresses(host, port).await).await else {
        return Vec::new();
    };
    let mut times = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        if connect(addr).await {
            times.push(u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX));
        }
    }
    times
}

/// Whether a TCP connection to one of the addresses of `host:port` succeeds.
#[must_use]
pub async fn open(host: &str, port: u16) -> bool {
    first_open(&addresses(host, port).await).await.is_some()
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[tokio::test]
    async fn times_a_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let times = connect_times("127.0.0.1", port, 3).await;
        assert_eq!(times.len(), 3);
        assert!(open("127.0.0.1", port).await);
        drop(listener);
        assert!(!open("127.0.0.1", port).await);
        assert_eq!(connect_times("127.0.0.1", port, 2).await.len(), 0);
    }

    #[tokio::test]
    async fn an_address_that_does_not_answer_is_passed_over() {
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let unreachable = closed.local_addr().unwrap();
        drop(closed);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let local = listener.local_addr().unwrap();
        assert_eq!(first_open(&[unreachable, local]).await, Some(local));
        assert_eq!(first_open(&[unreachable]).await, None);
        assert_eq!(first_open(&[]).await, None);
    }
}
