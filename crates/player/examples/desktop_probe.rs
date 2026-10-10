//! X1: prueba guiada de la aplicación real; no abre sesión ni imprime secretos.
use std::io;
use std::path::PathBuf;
use std::process::Command;

fn wait(text: &str) -> io::Result<()> {
    println!("{text}\nPulsa Enter para continuar.");
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(())
}

fn main() -> io::Result<()> {
    let executable = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("Uso: desktop_probe <ruta a velo.exe compilado con X1>"))?;
    let mut first = Command::new(&executable).spawn()?;
    wait("Espera a que abra Velo. Si ya estaba abierto, usa esa primera ventana. Minimízala.")?;
    for args in [
        vec![],
        vec!["spotify:track:4uLU6hMCjMI75M1A2tKUQC"],
        vec!["https://open.spotify.com/intl-es/album/1DFixLWuPkv3KT3TnV35m3?si=probe"],
        vec!["spotify:artist:4Z8W4fKeB5YxbusRsdQVPb"],
        vec!["spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"],
        vec![
            "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
            "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
        ],
        vec!["https://open.spotify.com.evil.test/track/ABC"],
    ] {
        let started = std::time::Instant::now();
        let mut second = Command::new(&executable).args(args).spawn()?;
        let status = loop {
            if let Some(status) = second.try_wait()? {
                break status;
            }
            if started.elapsed() > std::time::Duration::from_secs(6) {
                // Solo terminamos la segunda copia que creó este probe, nunca Velo ajeno.
                let _ = second.kill();
                let _ = second.wait();
                return Err(io::Error::other(
                    "La segunda ejecución no salió en 6 s; revisa que el exe incluya X1",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        if !status.success() {
            return Err(io::Error::other(
                "La segunda ejecución no terminó correctamente en el plazo esperado",
            ));
        }
        println!(
            "Segunda ejecución terminada en {} ms.",
            started.elapsed().as_millis()
        );
        wait(
            "Comprueba foco/un solo proceso; luego reproducción, página, duplicados en cola o rechazo, según el caso.",
        )?;
    }
    wait(
        "Pega dos enlaces de canción en la ventana y después texto en el buscador. Comprueba orden y pegado normal.",
    )?;
    wait(
        "Cierra la ventana con música: debe ocultarse y seguir sonando. Prueba el menú de bandeja y Mostrar Velo.",
    )?;
    wait(
        "Pausa desde bandeja. Mide CPU/RAM durante 130 s; comprueba visualizador detenido. Prueba Salir.",
    )?;
    if first.try_wait()?.is_none() {
        return Err(io::Error::other(
            "El proceso inicial sigue abierto: completa Salir antes de repetir",
        ));
    }
    println!("Probe completado; los resultados auditivos/de recursos son comprobaciones manuales.");
    Ok(())
}
