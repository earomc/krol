//! The krol server binary: a drop-in replacement for `java -jar minecraft_server.jar`.
//!
//! Run it in a server directory; it reads and writes the same files as vanilla 1.8.8.

mod logging;

use std::io::BufRead;
use std::net::SocketAddr;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use krol_server::config::{PropertyManager, ServerProperties, eula};
use krol_server::net::{NetShared, handle_connection};
use krol_server::status::{StatusResponse, load_favicon};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{info, warn};

fn main() -> ExitCode {
    if let Err(e) = logging::init(Path::new(".")) {
        eprintln!("Failed to initialize logging: {e}");
        return ExitCode::FAILURE;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("Netty IO")
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to start runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run());
    info!("Stopping server");
    ExitCode::SUCCESS
}

fn game_type_name(id: i32) -> &'static str {
    match id {
        1 => "CREATIVE",
        2 => "ADVENTURE",
        3 => "SPECTATOR",
        // Unknown ids fall back to survival, as in vanilla.
        _ => "SURVIVAL",
    }
}

/// Starts the server and runs until it is stopped. Returning means "stop".
async fn run() {
    info!("Starting minecraft server version 1.8.8");
    info!("Loading properties");
    let mut pm = PropertyManager::open("server.properties");
    if !eula::has_accepted(Path::new("eula.txt")) {
        info!(
            "You need to agree to the EULA in order to run the server. Go to eula.txt for more info."
        );
        eula::create(Path::new("eula.txt"));
        return;
    }
    let props = ServerProperties::read(&mut pm);
    info!("Default game type: {}", game_type_name(props.gamemode));
    // TODO(M2): generate the RSA key pair used for online-mode encryption.
    info!("Generating keypair");

    let host = if props.server_ip.is_empty() {
        "*"
    } else {
        props.server_ip.as_str()
    };
    info!("Starting Minecraft server on {host}:{}", props.server_port);
    let bind_ip = if props.server_ip.is_empty() {
        "0.0.0.0"
    } else {
        props.server_ip.as_str()
    };
    let listener = match bind(bind_ip, props.server_port).await {
        Ok(l) => l,
        Err(e) => {
            warn!("**** FAILED TO BIND TO PORT!");
            warn!("The exception was: java.net.BindException: {e}");
            warn!("Perhaps a server is already running on that port?");
            return;
        }
    };
    if cfg!(target_os = "linux") && props.use_native_transport {
        info!("Using epoll channel type");
    } else {
        info!("Using default channel type");
    }

    if !props.online_mode {
        warn!("**** SERVER IS RUNNING IN OFFLINE/INSECURE MODE!");
        warn!("The server will make no attempt to authenticate usernames. Beware.");
        warn!(
            "While this makes the game possible to play without internet access, it also opens up the ability for hackers to connect with any username they choose."
        );
        warn!("To change this, set \"online-mode\" to \"true\" in the server.properties file.");
    }

    let started = Instant::now();
    // TODO(M3): load worlds ("Preparing level ..." / "Preparing start region ...").
    let shared = NetShared::new(StatusResponse {
        motd: props.motd.clone(),
        max_players: props.max_players,
        online_players: 0,
        sample: Vec::new(),
        favicon: load_favicon(Path::new("server-icon.png")),
    });
    info!(
        "Done ({:.3}s)! For help, type \"help\" or \"?\"",
        started.elapsed().as_secs_f64()
    );

    let mut commands = spawn_console_reader();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((socket, _addr)) => {
                    let _ = socket.set_nodelay(true);
                    let shared = shared.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(socket, shared).await {
                            tracing::debug!("connection closed: {e}");
                        }
                    });
                }
                Err(e) => warn!("Failed to accept connection: {e}"),
            },
            command = commands.recv() => match command {
                Some(line) => {
                    if run_console_command(&line) {
                        return;
                    }
                }
                // Console closed (EOF): keep running, like vanilla.
                None => commands = futures_pending(),
            },
            _ = tokio::signal::ctrl_c() => return,
        }
    }
}

async fn bind(ip: &str, port: i32) -> std::io::Result<TcpListener> {
    let port = u16::try_from(port)
        .map_err(|_| std::io::Error::other(format!("Port value out of range: {port}")))?;
    let addr: SocketAddr = tokio::net::lookup_host((ip, port))
        .await?
        .next()
        .ok_or_else(|| std::io::Error::other(format!("{ip}: Name or service not known")))?;
    TcpListener::bind(addr).await
}

/// Handles one console line; returns `true` if the server should stop.
fn run_console_command(line: &str) -> bool {
    let command = line.trim().trim_start_matches('/');
    let name = command.split(' ').next().unwrap_or("");
    match name {
        "" => false,
        "stop" => {
            info!("Stopping the server");
            true
        }
        _ => {
            info!("Unknown command. Try /help for a list of commands");
            false
        }
    }
}

fn spawn_console_reader() -> mpsc::UnboundedReceiver<String> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("Server console handler".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            thread = "Server console handler",
                            "Exception handling console input: {e}"
                        );
                        break;
                    }
                }
            }
        })
        .expect("spawn console thread");
    rx
}

/// A receiver that never yields, used once stdin is closed.
fn futures_pending() -> mpsc::UnboundedReceiver<String> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::mem::forget(tx);
    rx
}
