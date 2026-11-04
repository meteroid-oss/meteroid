use std::net::SocketAddr;

pub fn free_local_socket() -> Option<SocketAddr> {
    let socket = std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0);
    std::net::TcpListener::bind(socket)
        .and_then(|listener| listener.local_addr())
        .ok()
}
pub fn free_local_port() -> Option<u16> {
    free_local_socket().map(|addr| addr.port())
}
