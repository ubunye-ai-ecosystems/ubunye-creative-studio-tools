//! kopanod: the Kopano service for this computer.
//!
//!   kopanod run    --home H [--api-port 47100]   run until stopped (start at login later, S4)
//!   kopanod import --home H --from <lab peer dir> take a lab project in (S1 stand-in for sign-in)
//!   kopanod call   --home H [--api-port P] --json '{"op":..}'   one request, as an app would
//!   kopanod watch  --home H [--api-port P]        print live events, as an app panel would

use anyhow::{Context, Result};
use kopano_node::{api_token, import_peer_dir, serve_api, Node};
use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let home = PathBuf::from(arg(&args, "--home").context("--home is required")?);
    let port: u16 = arg(&args, "--api-port").map(|p| p.parse()).transpose()?.unwrap_or(47100);
    match args.get(1).map(String::as_str) {
        Some("run") => {
            let node = Node::open(&home)?;
            api_token(&home)?; // before going online, so an app can read it straight away
            let router = node.start().await?;
            eprintln!("[kopanod] {} online as {}, {} project(s), app port {port}", node.device.cert.body.name, &node.device.id()[..8], node.projects.read().unwrap().len());
            tokio::select! {
                r = serve_api(node.clone(), port) => r?,
                _ = tokio::signal::ctrl_c() => {}
            }
            router.shutdown().await?;
            Ok(())
        }
        Some("import") => {
            let pid = import_peer_dir(&home, &PathBuf::from(arg(&args, "--from").context("--from")?))?;
            println!("{{\"imported\":\"{pid}\"}}");
            Ok(())
        }
        Some(op @ ("call" | "watch")) => {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let token = api_token(&home)?;
            let mut req: serde_json::Value = if op == "call" { serde_json::from_str(&arg(&args, "--json").context("--json")?)? } else { serde_json::json!({"op": "subscribe"}) };
            req["token"] = token.into();
            let c = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.context("kopanod is not running")?;
            let (r, mut w) = c.into_split();
            w.write_all(format!("{req}\n").as_bytes()).await?;
            let mut lines = BufReader::new(r).lines();
            while let Some(l) = lines.next_line().await? {
                println!("{l}");
                if op == "call" {
                    break;
                }
            }
            Ok(())
        }
        _ => anyhow::bail!("usage: kopanod run|import|call|watch --home H (see the file header)"),
    }
}
