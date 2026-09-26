// Demarrage / arret de la pile wrapper-lite : VM colima + conteneur Docker.
//
// Utilise par le raccourci Ctrl+S / F2 de l'interface. Toute la sortie des
// commandes est streamée ligne par ligne vers l'appelant pour l'afficher.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub const PROFILE: &str = "amdl";
pub const CONTAINER: &str = "wrapper-lite";
pub const MEMORY: &str = "1.5";
pub const LITE_STATUS: &str = "http://127.0.0.1:12340/status";

fn path_env() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    format!("{home}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
}

/// Socket Docker de la VM colima.
///
/// On passe par DOCKER_HOST plutot que par `docker context use` : le contexte
/// `colima-amdl` disparait quand la VM redemarre, et les commandes docker
/// retombent alors silencieusement sur le socket par defaut (daemon absent).
fn docker_host() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    format!("unix://{home}/.colima/{PROFILE}/docker.sock")
}

/// wrapper-lite repond-il sur /status ?
pub fn lite_up() -> bool {
    ureq::agent()
        .get(LITE_STATUS)
        .config()
        .timeout_global(Some(Duration::from_secs(3)))
        .build()
        .call()
        .is_ok()
}

/// La VM colima du profil est-elle démarrée ?
pub fn vm_running() -> bool {
    Command::new("colima")
        .args(["status", "--profile", PROFILE])
        .env("PATH", path_env())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn container_running() -> bool {
    match Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("name=^/{CONTAINER}$"),
            "--format",
            "{{.Names}}",
        ])
        .env("PATH", path_env())
        .env("DOCKER_HOST", docker_host())
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim() == CONTAINER,
        Err(_) => false,
    }
}

fn stream<R: Read + Send + 'static>(r: R, tx: Sender<String>) {
    std::thread::spawn(move || {
        let mut rd = r;
        let mut buf = [0u8; 4096];
        let mut acc: Vec<u8> = Vec::new();
        loop {
            match rd.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for &b in &buf[..n] {
                        if b == b'\n' || b == b'\r' {
                            if !acc.is_empty() {
                                let s = String::from_utf8_lossy(&acc).trim().to_string();
                                acc.clear();
                                if !s.is_empty() {
                                    let _ = tx.send(s);
                                }
                            }
                        } else {
                            acc.push(b);
                        }
                    }
                }
                Err(_) => break,
            }
        }
        if !acc.is_empty() {
            let s = String::from_utf8_lossy(&acc).trim().to_string();
            if !s.is_empty() {
                let _ = tx.send(s);
            }
        }
    });
}

fn cmd(program: &str, args: &[&str], tx: &Sender<String>) -> Result<i32, String> {
    let mut child = Command::new(program)
        .args(args)
        .env("PATH", path_env())
        .env("DOCKER_HOST", docker_host())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program} : {e}"))?;

    if let Some(o) = child.stdout.take() {
        stream(o, tx.clone());
    }
    if let Some(e) = child.stderr.take() {
        stream(e, tx.clone());
    }
    let st = child.wait().map_err(|e| format!("{program} : {e}"))?;
    Ok(st.code().unwrap_or(-1))
}

/// Demarre la VM puis le conteneur, et attend que /status reponde.
pub fn start(tx: &Sender<String>) -> Result<(), String> {
    if lite_up() {
        let _ = tx.send("wrapper-lite est deja en marche.".into());
        return Ok(());
    }

    if vm_running() {
        let _ = tx.send("VM colima deja demarree.".into());
    } else {
        let _ = tx.send(format!(
            "Demarrage de la VM colima ({MEMORY} Go de RAM) — environ 1 minute..."
        ));
        let code = cmd(
            "colima",
            &["start", "--profile", PROFILE, "--memory", MEMORY],
            tx,
        )?;
        if code != 0 {
            return Err(format!("colima start a echoue (code {code})"));
        }
    }

    if container_running() {
        let _ = tx.send("Conteneur deja en marche.".into());
    } else {
        let _ = tx.send("Demarrage du conteneur wrapper-lite...".into());
        let started = cmd("docker", &["start", CONTAINER], tx).unwrap_or(-1);
        if started != 0 {
            let _ = tx.send("Conteneur absent : creation...".into());
            let code = cmd(
                "docker",
                &[
                    "run",
                    "-d",
                    "--name",
                    CONTAINER,
                    "--privileged",
                    "--platform",
                    "linux/amd64",
                    "-p",
                    "12340:12340",
                    "-v",
                    "wl-data:/app/rootfs/data",
                    "--entrypoint",
                    "/app/wrapper-lite-rootless",
                    "wrapper-lite:local",
                    "--base-dir",
                    "/data",
                    "--host",
                    "0.0.0.0",
                    "--port",
                    "12340",
                ],
                tx,
            )?;
            if code != 0 {
                return Err(format!("creation du conteneur echouee (code {code})"));
            }
        }
    }

    let _ = tx.send("Attente de l'API sur le port 12340...".into());
    let deadline = Instant::now() + Duration::from_secs(150);
    while Instant::now() < deadline {
        if lite_up() {
            let _ = tx.send("wrapper-lite est pret.".into());
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    Err("wrapper-lite ne repond toujours pas apres 150 s".into())
}

/// Arrete le conteneur puis la VM (ce qui libere ~1,7 Go de RAM).
pub fn stop(tx: &Sender<String>) -> Result<(), String> {
    if container_running() {
        let _ = tx.send("Arret du conteneur wrapper-lite...".into());
        cmd("docker", &["stop", CONTAINER], tx)?;
    } else {
        let _ = tx.send("Conteneur deja arrete.".into());
    }

    if vm_running() {
        let _ = tx.send("Arret de la VM colima (liberation de la RAM)...".into());
        let code = cmd("colima", &["stop", "--profile", PROFILE], tx)?;
        if code != 0 {
            return Err(format!("colima stop a echoue (code {code})"));
        }
    }
    let _ = tx.send("Pile arretee.".into());
    Ok(())
}

/// Bascule : arrete si ca tourne, demarre sinon. Renvoie true si en marche.
pub fn toggle(tx: &Sender<String>) -> Result<bool, String> {
    if lite_up() {
        stop(tx)?;
        Ok(false)
    } else {
        start(tx)?;
        Ok(true)
    }
}
