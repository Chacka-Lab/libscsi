use clap::{Parser, Subcommand};
use libscsi::{Cdb, Direction, OpenOpts, ScsiCommand, ScsiDevice};

#[derive(Parser, Debug)]
#[command(
    name = "scsi-tool",
    version = "v0.0.2",
    about = "SCSI Command Tool based on libscsi."
)]
struct Cli {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand, Debug)]
enum Mode {
    /// Direct Mode
    Execute {
        /// The device path for opening, similar to: /dev/sg0 or /dev/sda.
        #[arg(short, long)]
        path: String,

        /// The command sent to the device. (Hex format, ignoring spaces)
        #[arg(short, long)]
        cdb: String,

        /// The data sent to the device for OUT commands. (Hex format, ignoring spaces)
        #[arg(short, long)]
        data: Option<String>,

        /// Pre-allocate a receive buffer of N bytes for IN commands.
        /// Use this instead of --data when reading from the device.
        #[arg(short, long)]
        alloc: Option<usize>,

        /// The data direction. (0:None, 1:In, 2:Out)
        #[arg(short = 'o', long)]
        dir: Option<u8>,

        /// Timeout duration (secs), must be a positive integer.
        #[arg(short, long, value_parser = parse_positive_u64)]
        timeout: Option<u32>,
    },

    /// Interactive Mode (Terminal)
    Term {
        /// Timeout duration (secs), must be a positive integer.
        #[arg(short, long, value_parser = parse_positive_u64)]
        timeout: Option<u32>,
    },
}

fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if cleaned.len() % 2 != 0 {
        return Err(format!("hex string has odd length: {}", cleaned.len()));
    }

    let mut out = Vec::with_capacity(cleaned.len() / 2);
    for pair in cleaned.as_bytes().chunks(2) {
        let hi = (pair[0] as char)
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex char: {}", pair[0] as char))?;
        let lo = (pair[1] as char)
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex char: {}", pair[1] as char))?;
        out.push(((hi << 4) | lo) as u8);
    }
    Ok(out)
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    if bytes.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(bytes.len() * 3 - 1);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 {
            result.push(' ');
        }
        result.push(HEX[(b >> 4) as usize] as char);
        result.push(HEX[(b & 0x0F) as usize] as char);
    }
    result
}

fn parse_positive_u64(s: &str) -> Result<u64, String> {
    let n: u64 = s.parse().map_err(|_| {
        "\"timeout\" param is not valid. It must be a positive integer.".to_string()
    })?;

    if n == 0 {
        Err("\"timeout\" param must be greater than 0.".to_string())
    } else {
        Ok(n)
    }
}
fn main() {
    let cli = Cli::parse();
    match cli.mode {
        Mode::Execute {
            path,
            cdb,
            data,
            alloc,
            dir,
            timeout,
        } => {
            let opts: OpenOpts = OpenOpts { exclusive: false };
            let mut device = match ScsiDevice::open(path.as_ref(), &opts) {
                Ok(device) => device,
                Err(e) => {
                    eprintln!("open device failed: {}", e);
                    std::process::exit(2);
                }
            };

            let cdb = match from_hex(&cdb) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("cdb hex: {}", e);
                    std::process::exit(2);
                }
            };
            let cdb = match Cdb::new(cdb) {
                Ok(cdb) => cdb,
                Err(e) => {
                    eprintln!("cdb failed: {}", e);
                    std::process::exit(2);
                }
            };

            let direction: Direction = match dir {
                None => Direction::None,
                Some(0) => Direction::None,
                Some(1) => Direction::In,
                Some(2) => Direction::Out,
                Some(_) => {
                    eprintln!("dir failed: not valid direction",);
                    std::process::exit(2);
                }
            };

            if data.is_some() && alloc.is_some() {
                eprintln!("--data and --alloc are mutually exclusive");
                std::process::exit(2);
            }

            let data: Vec<u8> = if let Some(n) = alloc {
                vec![0u8; n]
            } else if let Some(s) = data {
                match from_hex(&s) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("data hex: {}", e);
                        std::process::exit(2);
                    }
                }
            } else {
                Vec::new()
            };
            let cmd: ScsiCommand = ScsiCommand {
                cdb,
                direction,
                data,
                timeout_secs: timeout,
            };

            match device.execute(cmd) {
                Ok(result) => {
                    println!("STATUS: {:?}", result.status);
                    if !result.sense.as_bytes().is_empty() {
                        println!("SENSE: {}", to_hex(result.sense.as_bytes()));
                    }
                    if !result.data.is_empty() {
                        println!("DATA: {}", to_hex(&result.data));
                    }
                }
                Err(error) => {
                    eprintln!("execute failed: {}", error);
                    std::process::exit(1);
                }
            }
        }
        Mode::Term { .. } => {
            todo!("Complete term mode")
        }
    }
}
