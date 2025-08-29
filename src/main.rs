use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use daemonize::Daemonize;
use ddccid::*;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Parser)]
#[command(name = "ddcutil-brightness")]
#[command(about = "Fast DDC/CI brightness control for waybar")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the daemon
    Daemon,
    /// Get current brightness as JSON for waybar
    Get,
    /// Increase brightness
    Up {
        #[arg(short, long, default_value_t = 5)]
        step: u16,
    },
    /// Decrease brightness
    Down {
        #[arg(short, long, default_value_t = 5)]
        step: u16,
    },
    /// Set absolute brightness value
    Set {
        #[arg(value_name = "VALUE")]
        value: u16,
    },
    /// Stop the daemon
    Stop,
}

const SOCKET_PATH: &str = "/tmp/ddccid.sock";
const PID_FILE: &str = "/tmp/ddccid.pid";
const STDOUT: &str = "/tmp/ddccid.log";
const STDERR: &str = "/tmp/ddccid.err";

fn format_result(res: Result<u16, impl AsRef<dyn std::error::Error>>) -> String {
    match res {
        Ok(val) => format!(
            "{{\"text\": \"{val}\", \"percentage\": {val}, \"tooltip\": \"Brightness: {val}%\"}}"
        ),
        Err(e) => format!(
            "{{\"text\": \"?\", \"percentage\": 0, \"tooltip\": \"Error: {}\"}}",
            e.as_ref()
        ),
    }
}

fn handle_client(mut stream: UnixStream, manager: Arc<Mutex<impl BrightnessManager>>) {
    let mut line = String::new();
    let mut reader = BufReader::new(&stream);

    let Ok(_) = reader.read_line(&mut line) else {
        return;
    };
    let command = line.trim();
    let response = match command {
        "get" => format_result(manager.lock().unwrap().get_brightness()),
        cmd if cmd.starts_with("up ") => {
            let step: u16 = cmd.strip_prefix("up ").unwrap_or("5").parse().unwrap_or(5);
            format_result(manager.lock().unwrap().adjust_brightness(step as i16))
        }
        cmd if cmd.starts_with("down ") => {
            let step: u16 = cmd
                .strip_prefix("down ")
                .unwrap_or("5")
                .parse()
                .unwrap_or(5);
            format_result(manager.lock().unwrap().adjust_brightness(-(step as i16)))
        }
        cmd if cmd.starts_with("set ") => {
            let value: u16 = cmd
                .strip_prefix("set ")
                .unwrap_or("50")
                .parse()
                .unwrap_or(50);
            format_result(manager.lock().unwrap().set_brightness(value))
        }
        "stop" => {
            let _ = writeln!(stream, "OK stopping");
            let _ = std::fs::remove_file(SOCKET_PATH);
            let _ = std::fs::remove_file(PID_FILE);
            process::exit(0);
        }
        _ => "ERROR unknown command".to_string(),
    };

    let _ = writeln!(stream, "{}", response);
}

#[cfg(feature = "ddc-hi")]
type Backend = DdcHiBackend;

#[cfg(feature = "ddcutil")]
type Backend = DdcutilBackend;

fn start_daemon() -> Result<(), anyhow::Error> {
    // Check if daemon is already running
    if Path::new(SOCKET_PATH).exists() {
        bail!("Daemon already running (socket exists)");
    }

    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(STDOUT)?;

    let stderr = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(STDERR)?;

    let daemonize = Daemonize::new()
        .pid_file(PID_FILE)
        .chown_pid_file(true)
        .working_directory("/tmp")
        .stdout(stdout)
        .stderr(stderr);

    match daemonize.start() {
        Ok(_) => {
            let manager = Arc::new(Mutex::new(Backend::new()?));
            let listener = UnixListener::bind(SOCKET_PATH)?;

            println!("Daemon started, listening on {}", SOCKET_PATH);

            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let manager_clone = Arc::clone(&manager);
                        thread::spawn(move || {
                            handle_client(stream, manager_clone);
                        });
                    }
                    Err(err) => {
                        eprintln!("Error accepting connection: {}", err);
                    }
                }
            }
        }
        Err(e) => bail!("Error daemonizing: {}", e),
    }

    Ok(())
}

fn send_command(command: &str) -> Result<String, anyhow::Error> {
    let mut stream = UnixStream::connect(SOCKET_PATH)?;
    writeln!(stream, "{}", command)?;

    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;

    Ok(response.trim().to_string())
}

fn main() {
    let cli = Cli::parse();

    let now = std::time::Instant::now();
    match cli.command {
        Commands::Daemon => {
            start_daemon().expect("Failed to start daemon");
            return;
        }
        Commands::Get => {
            match send_command("get") {
                Ok(response) => println!("{}", response),
                Err(_) => {
                    // Fallback to direct mode if daemon not running
                    eprintln!("Daemon not running, getting brightness directly");
                    println!(
                        "{}",
                        format_result(Backend::new().and_then(|m| m.get_brightness()))
                    );
                }
            }
        }
        Commands::Up { step } => {
            match send_command(&format!("up {}", step)) {
                Ok(response) => println!("{}", response),
                Err(_) => {
                    // Fallback to direct mode
                    eprintln!("Daemon not running, adjusting brightness directly");
                    println!(
                        "{}",
                        format_result(
                            Backend::new().and_then(|m| m.adjust_brightness(step as i16))
                        )
                    );
                }
            }
        }
        Commands::Down { step } => {
            match send_command(&format!("down {}", step)) {
                Ok(response) => println!("{}", response),
                Err(_) => {
                    // Fallback to direct mode
                    eprintln!("Daemon not running, adjusting brightness directly");
                    println!(
                        "{}",
                        format_result(
                            Backend::new().and_then(|m| m.adjust_brightness(-(step as i16)))
                        )
                    );
                }
            }
        }
        Commands::Set { value } => {
            match send_command(&format!("set {}", value)) {
                Ok(response) => println!("{}", response),
                Err(_) => {
                    // Fallback to direct mode
                    eprintln!("Daemon not running, setting brightness directly");
                    println!(
                        "{}",
                        format_result(Backend::new().and_then(|m| m.set_brightness(value)))
                    );
                }
            }
        }
        Commands::Stop => {
            match send_command("stop") {
                Ok(response) => println!("{}", response),
                Err(e) => {
                    eprintln!("Error stopping daemon: {}", e);
                    // Try to clean up files anyway
                    let _ = std::fs::remove_file(SOCKET_PATH);
                    let _ = std::fs::remove_file(PID_FILE);
                }
            }
        }
    }
    let elapsed = now.elapsed();
    println!("Elapsed time: {:?}", elapsed);
}
