use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use ddcutil::{DisplayInfo, DisplayInfoList};
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
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

const SOCKET_PATH: &str = "/tmp/ddcutil-brightness.sock";
const PID_FILE: &str = "/tmp/ddcutil-brightness.pid";

struct BrightnessManager {
    displays: DisplayInfoList,
}

impl BrightnessManager {
    fn new() -> Result<Self> {
        let displays = DisplayInfo::enumerate()?;
        if displays.len() == 0 {
            bail!("No DDC/CI-capable displays found")
        }
        Ok(BrightnessManager { displays })
    }

    fn get_brightness(&self) -> Result<u16> {
        let brightness = self.displays.get(0).open()?.vcp_get_value(10)?;
        Ok(brightness.value())
    }

    fn adjust_brightness(&self, delta: i16) -> Result<u16> {
        let current = self.get_brightness()?;
        let new_value = if delta < 0 {
            current.saturating_sub((-delta) as u16)
        } else {
            current.saturating_add(delta as u16)
        };

        self.set_brightness(new_value)
    }

    fn set_brightness(&self, value: u16) -> Result<u16> {
        let clamped_value = std::cmp::min(100, value);
        std::thread::scope(|s| {
            for display in &self.displays {
                s.spawn(move || {
                    let display = display.open()?;
                    display.vcp_set_raw(10, clamped_value)?;
                    Ok::<(), anyhow::Error>(())
                });
            }
        });
        Ok(clamped_value)
    }
}

fn handle_client(mut stream: UnixStream, manager: Arc<Mutex<BrightnessManager>>) {
    let mut line = String::new();

    let Ok(_) = stream.read_to_string(&mut line) else {
        return;
    };
    let command = line.trim();
    let response = match command {
        "get" => match manager.lock().unwrap().get_brightness() {
            Ok(brightness) => json!({
                "text": brightness.to_string(),
                "percentage": brightness,
                "tooltip": format!("Brightness: {}%", brightness)
            })
            .to_string(),
            Err(_) => json!({
                "text": "?",
                "percentage": 0,
                "tooltip": "Error reading brightness"
            })
            .to_string(),
        },
        cmd if cmd.starts_with("up ") => {
            let step: u16 = cmd.strip_prefix("up ").unwrap_or("5").parse().unwrap_or(5);
            match manager.lock().unwrap().adjust_brightness(step as i16) {
                Ok(new_brightness) => format!("OK {}", new_brightness),
                Err(e) => format!("ERROR {}", e),
            }
        }
        cmd if cmd.starts_with("down ") => {
            let step: u16 = cmd
                .strip_prefix("down ")
                .unwrap_or("5")
                .parse()
                .unwrap_or(5);
            match manager.lock().unwrap().adjust_brightness(-(step as i16)) {
                Ok(new_brightness) => format!("OK {}", new_brightness),
                Err(e) => format!("ERROR {}", e),
            }
        }
        cmd if cmd.starts_with("set ") => {
            let value: u16 = cmd
                .strip_prefix("set ")
                .unwrap_or("50")
                .parse()
                .unwrap_or(50);
            match manager.lock().unwrap().set_brightness(value) {
                Ok(new_brightness) => format!("OK {}", new_brightness),
                Err(e) => format!("ERROR {}", e),
            }
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

fn start_daemon() -> Result<(), Box<dyn std::error::Error>> {
    // Check if daemon is already running
    if Path::new(SOCKET_PATH).exists() {
        return Err("Daemon already running (socket exists)".into());
    }

    // Write PID file
    std::fs::write(PID_FILE, process::id().to_string())?;

    let manager = Arc::new(Mutex::new(BrightnessManager::new()?));
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

    Ok(())
}

fn send_command(command: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(SOCKET_PATH)?;
    writeln!(stream, "{}", command)?;

    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;

    Ok(response.trim().to_string())
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon => {
            if let Err(e) = start_daemon() {
                eprintln!("Error starting daemon: {}", e);
                process::exit(1);
            }
        }
        Commands::Get => {
            match send_command("get") {
                Ok(response) => println!("{}", response),
                Err(_) => {
                    // Fallback to direct mode if daemon not running
                    match BrightnessManager::new().and_then(|m| m.get_brightness()) {
                        Ok(brightness) => {
                            let output = json!({
                                "text": brightness.to_string(),
                                "percentage": brightness,
                                "tooltip": format!("Brightness: {}%", brightness)
                            });
                            println!("{}", output);
                        }
                        Err(e) => {
                            eprintln!("Error: {}", e);
                            process::exit(1);
                        }
                    }
                }
            }
        }
        Commands::Up { step } => {
            match send_command(&format!("up {}", step)) {
                Ok(_) => {}
                Err(_) => {
                    // Fallback to direct mode
                    if let Ok(manager) = BrightnessManager::new() {
                        let _ = manager.adjust_brightness(step as i16);
                    }
                }
            }
        }
        Commands::Down { step } => {
            match send_command(&format!("down {}", step)) {
                Ok(_) => {}
                Err(_) => {
                    // Fallback to direct mode
                    if let Ok(manager) = BrightnessManager::new() {
                        let _ = manager.adjust_brightness(-(step as i16));
                    }
                }
            }
        }
        Commands::Set { value } => {
            match send_command(&format!("set {}", value)) {
                Ok(_) => {}
                Err(_) => {
                    // Fallback to direct mode
                    if let Ok(manager) = BrightnessManager::new() {
                        let _ = manager.set_brightness(value);
                    }
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
}
