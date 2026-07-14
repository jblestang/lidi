use crate::aux::{self, udp};
use std::{
    io::{Read, Write},
    net,
    os::unix,
};

fn receive_udp<D>(
    config: &udp::Config<aux::DiodeReceive>,
    mut diode: D,
    to_udp_bind: net::SocketAddr,
    to_udp: net::SocketAddr,
) -> Result<usize, udp::Error>
where
    D: Read + Write,
{
    log::debug!("binding UDP socket to {to_udp_bind}");

    let client = net::UdpSocket::bind(to_udp_bind)?;

    let mut buffer = vec![0; config.buffer_size];

    loop {
        let header = udp::protocol::Header::deserialize_from(&mut diode)?;

        log::trace!(
            "received header for datagram, reading {} bytes",
            header.size
        );

        validate_datagram_size(header.size, config.buffer_size)?;

        diode.read_exact(&mut buffer[0..header.size])?;

        log::trace!("sending datagram to {to_udp}");

        client.send_to(&buffer[0..header.size], to_udp)?;
    }
}

fn receive_unix_loop(
    config: &udp::Config<aux::DiodeReceive>,
    to_udp_bind: net::SocketAddr,
    to_udp: net::SocketAddr,
    server: &unix::net::UnixListener,
) -> Result<(), udp::Error> {
    loop {
        let (client, client_addr) = server.accept()?;
        log::info!(
            "new Unix client ({}) connected",
            client_addr
                .as_pathname()
                .map_or("unknown".to_string(), |p| p.display().to_string())
        );
        match receive_udp(config, client, to_udp_bind, to_udp) {
            Ok(total) => log::info!("UDP received, {total} bytes received"),
            Err(e) => log::error!("failed to receive UDP: {e}"),
        }
    }
}

fn receive_tcp_loop(
    config: &udp::Config<aux::DiodeReceive>,
    to_udp_bind: net::SocketAddr,
    to_udp: net::SocketAddr,
    server: &net::TcpListener,
) -> Result<(), udp::Error> {
    loop {
        let (client, client_addr) = server.accept()?;
        log::info!("new Unix client ({client_addr}) connected");
        match receive_udp(config, client, to_udp_bind, to_udp) {
            Ok(total) => log::info!("UDP received, {total} bytes received"),
            Err(e) => log::error!("failed to receive UDP: {e}"),
        }
    }
}

/// # Errors
///
/// Will return `Err` if `from_unix` `PathBuf`
/// already exists.
pub fn receive(
    config: &udp::Config<aux::DiodeReceive>,
    to_udp_bind: net::SocketAddr,
    to_udp: net::SocketAddr,
) -> Result<(), udp::Error> {
    if let Some(from_unix) = &config.diode.from_unix {
        if from_unix.exists() {
            return Err(udp::Error::Other(format!(
                "Unix socket path '{}' already exists",
                from_unix.display()
            )));
        }

        let server = unix::net::UnixListener::bind(from_unix)?;
        receive_unix_loop(config, to_udp_bind, to_udp, &server)?;
    }

    if let Some(from_tcp) = &config.diode.from_tcp {
        let server = net::TcpListener::bind(from_tcp)?;
        receive_tcp_loop(config, to_udp_bind, to_udp, &server)?;
    }

    Ok(())
}

fn validate_datagram_size(size: usize, buffer_size: usize) -> Result<(), udp::Error> {
    if size == 0 || size > buffer_size {
        return Err(udp::Error::Other(format!(
            "invalid datagram size {size} (buffer size is {buffer_size})"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod repro {
    use super::{receive_udp, validate_datagram_size};
    use crate::aux::{self, udp};
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// Unpatched code indexes `buffer[0..header.size]` before checking bounds,
    /// which panics when `header.size > buffer.len()`.
    #[test]
    fn repro_oversized_datagram_slice_panics_without_bounds_check() {
        const BUFFER_SIZE: usize = 1024;
        let buffer = vec![0u8; BUFFER_SIZE];
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = &buffer[0..BUFFER_SIZE + 1];
        }));
        assert!(
            panic.is_err(),
            "oversized slice must panic on unpatched path"
        );
    }

    #[test]
    fn repro_zero_and_oversized_datagram_sizes_rejected() {
        const BUFFER_SIZE: usize = 1024;
        assert!(validate_datagram_size(0, BUFFER_SIZE).is_err());
        assert!(validate_datagram_size(BUFFER_SIZE + 1, BUFFER_SIZE).is_err());
        assert!(validate_datagram_size(512, BUFFER_SIZE).is_ok());
    }

    #[test]
    fn repro_malicious_udp_header_returns_error_without_panic() {
        const BUFFER_SIZE: usize = 1024;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");

        let config = udp::Config {
            diode: aux::DiodeReceive {
                from_tcp: None,
                from_unix: None,
            },
            buffer_size: BUFFER_SIZE,
        };
        let to_udp_bind = "127.0.0.1:0".parse().expect("udp bind addr");
        let to_udp = "127.0.0.1:9".parse().expect("udp dest addr");

        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let result = receive_udp(&config, stream, to_udp_bind, to_udp);
            let _ = done_tx.send(result);
        });

        let mut client = None;
        for _ in 0..100 {
            match std::net::TcpStream::connect(addr) {
                Ok(stream) => {
                    client = Some(stream);
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
        let mut client = client.expect("connect to udp relay");

        let bad_size = (BUFFER_SIZE + 1) as u64;
        client
            .write_all(&bad_size.to_le_bytes())
            .expect("send malicious udp header");

        match done_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Err(error)) => {
                assert!(
                    error.to_string().contains("invalid datagram size"),
                    "unexpected error: {error}"
                );
            }
            Ok(Ok(_)) => panic!("expected invalid datagram size error"),
            Err(_) => panic!("worker timed out"),
        }
        worker.join().expect("worker must not panic");
    }
}
