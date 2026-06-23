//! 网络地址解析工具。

use std::net::{SocketAddr, ToSocketAddrs};

/// 把 "host:port" 解析为 SocketAddr。host 可以是 IP，也可以是域名——标准库的
/// `SocketAddr::from_str`（即 `str::parse::<SocketAddr>()`）只认数字 IP、不解析
/// 域名，连接堡垒机/带域名的主机时会报 "invalid socket address syntax"。这里用
/// `to_socket_addrs` 走 DNS 解析，IP 与域名都支持，取首个解析结果。
pub fn resolve_addr(addr: &str) -> Result<SocketAddr, String> {
    addr.to_socket_addrs()
        .map_err(|e| format!("无法解析地址 {}: {}", addr, e))?
        .next()
        .ok_or_else(|| format!("地址解析为空: {}", addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_ip_literal() {
        let a = resolve_addr("127.0.0.1:2222").unwrap();
        assert_eq!(a.port(), 2222);
        assert!(a.ip().is_loopback());
    }

    #[test]
    fn test_resolve_invalid() {
        assert!(resolve_addr("not a valid addr").is_err());
    }
}
