//! Instancia única: bloqueo del SO y entrega autenticada exclusivamente en loopback.
//! Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino).

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

const MAX_FRAME: usize = 32 * 1024;
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    token: String,
    args: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    pid: u32,
    token: String,
}

pub enum Outcome {
    First(Guard, mpsc::Receiver<Vec<String>>),
    Forwarded,
    /// Hay otra ejecución con el bloqueo, pero no aceptó la entrega a tiempo.
    Busy,
}

pub struct Guard {
    // El archivo permanece: borrarlo permitiría bloquear dos inodos distintos en Unix.
    _lock: File,
    endpoint: PathBuf,
    address: SocketAddr,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        let _ = TcpStream::connect_timeout(&self.address, TIMEOUT);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.endpoint);
    }
}

/// Argumentos de esta ejecución. Desmedidos no impiden abrir ni mostrar Velo: se descartan.
pub fn launch_args() -> Vec<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if validate_args(&args).is_ok() {
        args
    } else {
        tracing::warn!("argumentos de inicio descartados por tamaño");
        Vec::new()
    }
}

pub fn acquire(args: Vec<String>) -> io::Result<Outcome> {
    let dirs = directories::ProjectDirs::from("", "Velo", "Velo")
        .ok_or_else(|| io::Error::other("No se encontró el directorio privado de Velo"))?;
    claim(&dirs.data_local_dir().join("instance"), args)
}

fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Bloqueo exclusivo que el SO libera al terminar el proceso, también tras un crash.
/// `Ok(None)`: otra ejecución lo tiene.
fn lock(path: &Path) -> io::Result<Option<File>> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Sin compartir: Windows niega cualquier otra apertura mientras viva el handle.
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path);
        match opened {
            Ok(file) => Ok(Some(file)),
            // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
            Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => Ok(None),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(windows))]
    {
        // Linux/macOS (pospuesto): `File::try_lock` exige Rust >= 1.89 en esas plataformas.
        let file = private_file(path)?;
        #[allow(clippy::incompatible_msrv)]
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(e),
        }
    }
}

fn claim(dir: &Path, args: Vec<String>) -> io::Result<Outcome> {
    validate_args(&args)?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let endpoint_path = dir.join("endpoint.json");
    let Some(lock) = lock(&dir.join("lock"))? else {
        // Solo al arrancar: otra ejecución puede estar publicando el endpoint.
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Ok(bytes) = std::fs::read(&endpoint_path)
                && let Ok(endpoint) = serde_json::from_slice::<Endpoint>(&bytes)
            {
                match forward(&endpoint, &args) {
                    Ok(()) => return Ok(Outcome::Forwarded),
                    // Solo se reintenta si todavía no se conectó. Una entrega ambigua
                    // nunca se repite: podría reproducir/encolar dos veces.
                    Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {}
                    Err(_) => return Ok(Outcome::Busy),
                }
            }
            if Instant::now() >= deadline {
                return Ok(Outcome::Busy);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    };
    // Lo primero: invalidar el endpoint de una ejecución anterior que terminó mal, para
    // que nadie entregue el token viejo a un puerto que ahora podría ser de otro proceso.
    let mut file = private_file(&endpoint_path)?;
    file.set_len(0)?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    let mut random = [0u8; 32];
    getrandom::fill(&mut random)
        .map_err(|_| io::Error::other("No se pudo crear el token local"))?;
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let endpoint = Endpoint {
        port: address.port(),
        pid: std::process::id(),
        token: token.clone(),
    };
    file.write_all(&serde_json::to_vec(&endpoint)?)?;
    file.sync_all()?;
    let (tx, rx) = mpsc::channel(32);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = stop.clone();
    let worker = std::thread::Builder::new()
        .name("velo-instance".into())
        .spawn(move || {
            for stream in listener.incoming() {
                if stopped.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                let mut stream = match stream {
                    Ok(stream) => stream,
                    // Un cliente que cortó antes de aceptarlo no detiene el canal.
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionReset
                        ) =>
                    {
                        continue;
                    }
                    Err(_) => break,
                };
                let _ = stream.set_write_timeout(Some(TIMEOUT));
                if let Ok(args) = read_socket_request(&mut stream, &token, TIMEOUT)
                    && tx.try_send(args).is_ok()
                {
                    let _ = stream.write_all(b"velo:ok\n");
                }
            }
        })?;
    Ok(Outcome::First(
        Guard {
            _lock: lock,
            endpoint: endpoint_path,
            address,
            stop,
            worker: Some(worker),
        },
        rx,
    ))
}

fn validate_args(args: &[String]) -> io::Result<()> {
    if args.len() > 100
        || args.iter().map(String::len).sum::<usize>() > MAX_FRAME / 2
        || args.iter().any(|arg| arg.chars().any(|c| c == '\0'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Argumentos demasiado grandes",
        ));
    }
    Ok(())
}

fn read_request(reader: impl Read, token: &str) -> io::Result<Vec<String>> {
    let mut frame = Vec::new();
    BufReader::new(reader)
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut frame)?;
    if frame.len() > MAX_FRAME || frame.last() != Some(&b'\n') {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Trama inválida"));
    }
    let request: Request = serde_json::from_slice(&frame)?;
    // Comparación sin salida temprana del token aleatorio, nunca registrado en logs.
    let same = request.token.len() == token.len()
        && request
            .token
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0;
    if request.version != 1 || !same {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Petición no autenticada",
        ));
    }
    validate_args(&request.args)?;
    Ok(request.args)
}

/// El timeout del socket limita cada read, no la trama entera. Un cliente sin token
/// que gotea bytes no puede retener el único receptor ni su join al salir de Velo.
fn read_socket_request(
    stream: &mut TcpStream,
    token: &str,
    timeout: Duration,
) -> io::Result<Vec<String>> {
    struct DeadlineReader<'a> {
        stream: &'a mut TcpStream,
        deadline: Instant,
    }
    impl Read for DeadlineReader<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Trama local vencida",
                ));
            }
            self.stream.set_read_timeout(Some(remaining))?;
            self.stream.read(buf)
        }
    }
    read_request(
        DeadlineReader {
            stream,
            deadline: Instant::now() + timeout,
        },
        token,
    )
}

fn forward(endpoint: &Endpoint, args: &[String]) -> io::Result<()> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port));
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn AllowSetForegroundWindow(pid: u32) -> i32;
        }
        // SAFETY: autoriza a la primera ejecución a tomar foco, sin tocar su memoria.
        unsafe {
            AllowSetForegroundWindow(endpoint.pid);
        }
    }
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let request = Request {
        version: 1,
        token: endpoint.token.clone(),
        args: args.to_vec(),
    };
    stream.write_all(&serde_json::to_vec(&request)?)?;
    stream.write_all(b"\n")?;
    let mut reply = [0; 8];
    stream.read_exact(&mut reply)?;
    if &reply != b"velo:ok\n" {
        return Err(io::Error::other("Respuesta local inválida"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_unauthenticated_client_cannot_extend_the_frame_deadline() {
        // Servidor mock local: cada byte llega antes del timeout individual, pero
        // la trama nunca acaba. Debe vencer el plazo total aun con datos llegando.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        let drip = std::thread::spawn(move || {
            for _ in 0..30 {
                if client.write_all(b" ").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let start = Instant::now();
        let error =
            read_socket_request(&mut server, "secreto", Duration::from_millis(150)).unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));
        assert!(start.elapsed() < Duration::from_millis(500));
        drop(server);
        drip.join().unwrap();
    }

    #[test]
    fn slow_client_does_not_block_the_next_launch_or_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let Outcome::First(guard, mut rx) = claim(dir.path(), vec![]).unwrap() else {
            panic!()
        };
        let mut client = TcpStream::connect(guard.address).unwrap();
        client.write_all(b"{").unwrap();
        let drip = std::thread::spawn(move || {
            for _ in 0..100 {
                std::thread::sleep(Duration::from_millis(40));
                if client.write_all(b" ").is_err() {
                    break;
                }
            }
        });
        // Deja margen entre el deadline del receptor y el de la segunda ejecución.
        std::thread::sleep(Duration::from_millis(500));
        assert!(matches!(
            claim(dir.path(), vec!["legítimo".into()]).unwrap(),
            Outcome::Forwarded
        ));
        assert_eq!(rx.blocking_recv().unwrap(), vec!["legítimo".to_string()]);
        let start = Instant::now();
        drop(guard);
        assert!(start.elapsed() < TIMEOUT);
        drip.join().unwrap();
    }

    #[test]
    fn protocol_rejects_bad_tokens_versions_and_oversized_frames() {
        let encode = |version, token: &str| {
            serde_json::to_vec(&Request {
                version,
                token: token.into(),
                args: vec!["spotify:track:ABC".into()],
            })
            .unwrap()
        };
        for (version, token) in [(1, "incorrecto"), (2, "secreto")] {
            let mut frame = encode(version, token);
            frame.push(b'\n');
            assert!(read_request(frame.as_slice(), "secreto").is_err());
        }
        assert!(read_request(encode(1, "secreto").as_slice(), "secreto").is_err());
        assert!(read_request(vec![b'a'; MAX_FRAME + 1].as_slice(), "secreto").is_err());
    }

    #[test]
    fn second_launch_forwards_in_order_and_crash_metadata_does_not_hold_lock() {
        let dir = tempfile::tempdir().unwrap();
        let Outcome::First(guard, mut rx) = claim(dir.path(), vec![]).unwrap() else {
            panic!()
        };
        assert!(guard.address.ip().is_loopback());
        let args = vec!["spotify:track:ABC".into(), "dos\ncon espacios".into()];
        assert!(matches!(
            claim(dir.path(), args.clone()).unwrap(),
            Outcome::Forwarded
        ));
        assert_eq!(rx.blocking_recv().unwrap(), args);
        assert!(matches!(
            claim(dir.path(), vec![]).unwrap(),
            Outcome::Forwarded
        ));
        assert!(rx.blocking_recv().unwrap().is_empty());
        drop(guard);
        std::fs::write(dir.path().join("endpoint.json"), b"obsoleto").unwrap();
        assert!(matches!(
            claim(dir.path(), vec![]).unwrap(),
            Outcome::First(..)
        ));
    }

    #[test]
    fn foreign_clients_get_no_reply_and_deliver_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let Outcome::First(guard, mut rx) = claim(dir.path(), vec![]).unwrap() else {
            panic!()
        };
        let endpoint: Endpoint =
            serde_json::from_slice(&std::fs::read(dir.path().join("endpoint.json")).unwrap())
                .unwrap();
        assert_eq!(endpoint.token.len(), 64);
        let bad = |payload: Vec<u8>| {
            let mut stream = TcpStream::connect(guard.address).unwrap();
            stream.set_read_timeout(Some(TIMEOUT * 2)).unwrap();
            let _ = stream.write_all(&payload);
            let _ = stream.shutdown(std::net::Shutdown::Write);
            let mut reply = Vec::new();
            let _ = stream.read_to_end(&mut reply);
            assert!(reply.is_empty());
        };
        bad(b"{\"version\":1,\"token\":\"00\",\"args\":[\"x\"]}
"
        .to_vec());
        bad(vec![b'{'; MAX_FRAME * 2]);
        assert!(rx.try_recv().is_err());
        // El canal sigue sirviendo a la ejecución legítima.
        assert!(matches!(
            claim(dir.path(), vec!["a".into()]).unwrap(),
            Outcome::Forwarded
        ));
        assert_eq!(rx.blocking_recv().unwrap(), vec!["a".to_string()]);
    }

    #[test]
    fn unresponsive_holder_is_busy_and_stale_endpoint_is_cleared_first() {
        let dir = tempfile::tempdir().unwrap();
        let held = lock(&dir.path().join("lock")).unwrap().unwrap();
        // Sin endpoint publicado ni respuesta: no se abre una segunda copia.
        assert!(matches!(claim(dir.path(), vec![]).unwrap(), Outcome::Busy));
        drop(held);
        let Outcome::First(_guard, _rx) = claim(dir.path(), vec![]).unwrap() else {
            panic!()
        };
        let endpoint: Endpoint =
            serde_json::from_slice(&std::fs::read(dir.path().join("endpoint.json")).unwrap())
                .unwrap();
        assert_eq!(endpoint.pid, std::process::id());
    }
}
