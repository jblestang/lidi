use crate::aux::{self, file};
use fasthash::HasherExt;
use std::{
    fs,
    hash::Hash,
    hash::Hasher,
    io::{Read, Write},
    net,
    os::unix::{self, fs::PermissionsExt},
    path, thread,
};

/// # Errors
///
/// Will return `Err` if `output_dir` is not a directory.
pub fn receive_files(
    config: &file::Config<aux::DiodeReceive>,
    output_dir: &path::Path,
) -> Result<(), file::Error> {
    if !output_dir.is_dir() {
        return Err(file::Error::Other(
            "output_directory is not a directory".to_string(),
        ));
    }

    thread::scope(|scope| -> Result<(), file::Error> {
        if let Some(from_unix) = &config.diode.from_unix {
            if from_unix.exists() {
                return Err(file::Error::Other(format!(
                    "Unix socket path '{}' already exists",
                    from_unix.display()
                )));
            }

            let server = unix::net::UnixListener::bind(from_unix)?;
            thread::Builder::new().spawn_scoped(scope, move || {
                receive_unix_loop(config, output_dir, scope, &server)
            })?;
        }

        if let Some(from_tcp) = &config.diode.from_tcp {
            let server = net::TcpListener::bind(from_tcp)?;
            thread::Builder::new().spawn_scoped(scope, move || {
                receive_tcp_loop(config, output_dir, scope, &server)
            })?;
        }

        Ok(())
    })
}

fn receive_tcp_loop<'a>(
    config: &'a file::Config<aux::DiodeReceive>,
    output_dir: &'a path::Path,
    scope: &'a thread::Scope<'a, '_>,
    server: &net::TcpListener,
) -> Result<(), file::Error> {
    let mut count = 0;

    loop {
        if config.max_files != 0 && count >= config.max_files {
            return Ok(());
        }
        count += 1;
        let (client, client_addr) = server.accept()?;
        log::info!("new TCP client ({client_addr}) connected");
        scope.spawn(|| match receive_file(config, client, output_dir) {
            Ok(total) => log::info!("file received, {total} bytes received"),
            Err(e) => log::error!("failed to receive file: {e}"),
        });
    }
}

fn receive_unix_loop<'a>(
    config: &'a file::Config<aux::DiodeReceive>,
    output_dir: &'a path::Path,
    scope: &'a thread::Scope<'a, '_>,
    server: &unix::net::UnixListener,
) -> Result<(), file::Error> {
    let mut count = 0;

    loop {
        if config.max_files != 0 && count >= config.max_files {
            return Ok(());
        }
        count += 1;
        let (client, client_addr) = server.accept()?;
        log::info!(
            "new Unix client ({}) connected",
            client_addr
                .as_pathname()
                .map_or("unknown".to_string(), |p| p.display().to_string())
        );
        scope.spawn(|| match receive_file(config, client, output_dir) {
            Ok(total) => log::info!("file received, {total} bytes received"),
            Err(e) => log::error!("failed to receive file: {e}"),
        });
    }
}

fn validate_file_name(file_name: &str) -> Result<(), file::Error> {
    if file_name.is_empty() || file_name == "." || file_name == ".." {
        return Err(file::Error::Other(format!(
            "invalid file name: {file_name:?}"
        )));
    }
    if file_name.contains('/') || file_name.contains('\\') {
        return Err(file::Error::Other(format!(
            "file name must not contain path separators: {file_name:?}"
        )));
    }
    Ok(())
}

fn output_file_path(output_dir: &path::Path, file_name: &str) -> Result<path::PathBuf, file::Error> {
    validate_file_name(file_name)?;

    let output_dir = output_dir
        .canonicalize()
        .map_err(file::Error::from)?;
    let file_path = output_dir.join(file_name);

    if !file_path.starts_with(&output_dir) {
        return Err(file::Error::Other(format!(
            "file path escapes output directory: {}",
            file_path.display()
        )));
    }

    Ok(file_path)
}

fn receive_file<D>(
    config: &file::Config<aux::DiodeReceive>,
    mut diode: D,
    output_dir: &path::Path,
) -> Result<usize, file::Error>
where
    D: Read + Write,
{
    let header = file::protocol::Header::deserialize_from(&mut diode)?;

    log::debug!("receiving file \"{}\"", header.file_name);
    log::debug!("file size = {}", header.file_length);

    let file_path = path::PathBuf::from(&header.file_name);
    let file_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(file::Error::Other("invalid file name".to_string()))?;
    let file_path = output_file_path(output_dir, file_name)?;

    log::debug!("storing at \"{}\"", file_path.display());

    if file_path.exists() {
        return Err(file::Error::Other(format!(
            "file \"{}\" already exists",
            file_path.display()
        )));
    }

    let mut file = fs::OpenOptions::new()
        .read(false)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&file_path)?;

    log::debug!("setting mode to {}", header.mode);
    file.set_permissions(fs::Permissions::from_mode(header.mode))?;

    let mut buffer = vec![0; config.buffer_size];
    let mut cursor = 0;
    let mut remaining = usize::try_from(header.file_length)?;

    let mut hasher = if config.hash {
        Some(fasthash::SpookyHasherExt::default())
    } else {
        None
    };

    loop {
        let end = if remaining >= (config.buffer_size - cursor) {
            config.buffer_size
        } else {
            cursor + remaining
        };
        match diode.read(&mut buffer[cursor..end])? {
            0 => {
                if 0 < cursor {
                    if let Some(hasher) = hasher.as_mut() {
                        hasher.write(&buffer[..cursor]);
                    }
                    file.write_all(&buffer[..cursor])?;
                }

                file.flush()?;

                let received = usize::try_from(header.file_length)? - remaining;

                let footer = file::protocol::Footer::deserialize_from(&mut diode)?;

                if remaining != 0 {
                    log::debug!("expected file size = {}", header.file_length);
                    log::debug!("received file size = {received}");
                    return Err(file::Error::Diode(file::protocol::Error::InvalidFileSize(
                        usize::try_from(header.file_length)?,
                        received,
                    )));
                }

                if let Some(hasher) = hasher.as_mut() {
                    let hash = hasher.finish_ext();
                    log::debug!("expected hash = {}", footer.hash);
                    log::debug!("computed hash = {hash}");
                    if footer.hash != hash {
                        return Err(file::Error::Diode(file::protocol::Error::InvalidHash(
                            hash,
                            footer.hash,
                        )));
                    }
                }

                return Ok(received);
            }
            nread => {
                remaining -= nread;
                if (cursor + nread) < config.buffer_size {
                    cursor += nread;
                    continue;
                }
                if let Some(hasher) = hasher.as_mut() {
                    buffer.hash(hasher);
                }
                file.write_all(&buffer)?;
                cursor = 0;
            }
        }
    }
}

#[cfg(test)]
mod repro {
    use super::{output_file_path, validate_file_name};
    use crate::aux::{self, file};
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    fn craft_file_header(file_name: &str) -> Vec<u8> {
        let mut bytes = file_name.len().to_le_bytes().to_vec();
        bytes.extend_from_slice(file_name.as_bytes());
        bytes.extend_from_slice(&0o644u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes
    }

    /// Before the fix, a sender could use `..` as the file name and write outside
    /// the configured output directory (`output_dir.join("..")` resolves upward).
    #[test]
    fn repro_parent_dir_filename_rejected() {
        assert!(validate_file_name("..").is_err());
        assert!(validate_file_name(".").is_err());
        assert!(validate_file_name("").is_err());

        let dir = tempfile::tempdir().expect("tempdir");
        assert!(output_file_path(dir.path(), "..").is_err());
    }

    #[test]
    fn repro_path_separator_in_filename_rejected() {
        assert!(validate_file_name("nested/evil.txt").is_err());
        assert!(validate_file_name("nested\\evil.txt").is_err());
    }

    #[test]
    fn repro_malicious_tcp_header_does_not_escape_output_dir() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        drop(listener);

        let sandbox = tempfile::tempdir().expect("tempdir");
        let parent = sandbox.path().to_path_buf();
        let output_dir = parent.join("inbox");
        std::fs::create_dir_all(&output_dir).expect("mkdir inbox");

        let config = file::Config {
            diode: aux::DiodeReceive {
                from_tcp: Some(addr),
                from_unix: None,
            },
            buffer_size: 4096,
            hash: false,
            max_files: 1,
        };

        let (done_tx, done_rx) = mpsc::channel();
        let output_for_receiver = output_dir.clone();
        let receiver = thread::spawn(move || {
            let result = file::receive::receive_files(&config, &output_for_receiver);
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
        let mut client = client.expect("receiver never accepted TCP connections");
        client
            .write_all(&craft_file_header(".."))
            .expect("send malicious header");
        drop(client);

        let receive_result = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("receiver timed out");
        assert!(
            receive_result.is_ok(),
            "receive_files failed: {}",
            receive_result.err().map_or_else(String::new, |e| e.to_string())
        );
        receiver.join().expect("receiver thread panicked");

        for entry in std::fs::read_dir(&parent).expect("read parent dir") {
            let entry = entry.expect("dir entry");
            assert_eq!(
                entry.file_name().to_string_lossy(),
                "inbox",
                "path traversal would create files outside inbox/"
            );
        }
    }
}
