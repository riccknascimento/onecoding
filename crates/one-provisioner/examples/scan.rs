//! Lê a fingerprint SHA-256 de um host SSH sem autenticar (fase 1 do provisionamento).
//! Uso: cargo run -p one-provisioner --example scan -- <host> <porta> <direct|tailscale>

use one_provisioner::guard::AccessType;
use one_provisioner::scan::scan_fingerprint;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("uso: scan <host> <porta> <direct|tailscale>");
        std::process::exit(2);
    }
    let port: u16 = args[2].parse().expect("porta inválida");
    let access = match args[3].as_str() {
        "direct" => AccessType::Direct,
        "tailscale" => AccessType::Tailscale,
        _ => {
            eprintln!("acesso deve ser direct ou tailscale");
            std::process::exit(2);
        }
    };
    match scan_fingerprint(&args[1], port, access).await {
        Ok(fp) => println!("{fp}"),
        Err(e) => {
            eprintln!("erro: {e}");
            std::process::exit(1);
        }
    }
}
