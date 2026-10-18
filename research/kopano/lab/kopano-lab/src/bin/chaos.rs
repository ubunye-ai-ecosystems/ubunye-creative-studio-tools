//! E18: a "bad network" between peers and a relay. TCP proxy that adds one-way delay to every
//! chunk (pipelined, so throughput is limited by bandwidth, not by the delay), caps bandwidth per
//! direction, and can drop every connection at intervals like a phone losing signal.
//!
//!   chaos --listen 127.0.0.1:4000 --to 127.0.0.1:3340 --kbps 1000 --delay-ms 150 [--drop-every-s 20]
//!
//! Prints one JSON line per dropped batch and a byte count on exit (Ctrl-C).

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

#[derive(Clone, Copy)]
struct Shape {
    bytes_per_s: f64,
    delay: Duration,
}

/// One direction: reader stamps chunks; writer releases each at stamp + delay, then paces to the bandwidth.
async fn pipe(mut r: tokio::net::tcp::OwnedReadHalf, mut w: tokio::net::tcp::OwnedWriteHalf, shape: Shape, counter: Arc<AtomicU64>, mut kill: broadcast::Receiver<()>) {
    let (tx, mut rx) = mpsc::channel::<(Instant, Vec<u8>)>(4096);
    let reader = tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send((Instant::now(), buf[..n].to_vec())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    // Windows wakes timers only every ~15 ms, so sleeping per chunk would add fake delay. The link
    // is modelled by a clock: each chunk leaves at max(arrival + delay, link free); we only sleep
    // when the next departure is more than 2 ms away, and send everything already due together.
    let mut link_free = Instant::now();
    let slack = Duration::from_millis(2);
    loop {
        tokio::select! {
            _ = kill.recv() => break,
            m = rx.recv() => {
                let Some((t, chunk)) = m else { break };
                let busy = Duration::from_secs_f64(chunk.len() as f64 / shape.bytes_per_s);
                let depart = (t + shape.delay).max(link_free) + busy;
                link_free = depart;
                let now = Instant::now();
                if depart > now + slack {
                    tokio::time::sleep(depart - now).await;
                }
                if w.write_all(&chunk).await.is_err() {
                    break;
                }
                counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            }
        }
    }
    reader.abort();
    let _ = w.shutdown().await;
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let listen = arg(&args, "--listen").unwrap_or_else(|| "127.0.0.1:4000".into());
    let to = arg(&args, "--to").context("--to")?;
    let kbps: f64 = arg(&args, "--kbps").map(|s| s.parse()).transpose()?.unwrap_or(1000.0);
    let delay_ms: u64 = arg(&args, "--delay-ms").map(|s| s.parse()).transpose()?.unwrap_or(0);
    let drop_every: u64 = arg(&args, "--drop-every-s").map(|s| s.parse()).transpose()?.unwrap_or(0);
    let shape = Shape { bytes_per_s: kbps * 1000.0 / 8.0, delay: Duration::from_millis(delay_ms) };
    let l = TcpListener::bind(&listen).await?;
    let (kill_tx, _) = broadcast::channel::<()>(16);
    let bytes = Arc::new(AtomicU64::new(0));
    if drop_every > 0 {
        let k = kill_tx.clone();
        tokio::spawn(async move {
            let mut n = 0u64;
            loop {
                tokio::time::sleep(Duration::from_secs(drop_every)).await;
                n += 1;
                let _ = k.send(());
                println!("{{\"event\":\"drop\",\"n\":{n}}}");
            }
        });
    }
    eprintln!("[chaos] {listen} -> {to}: {kbps} kbit/s each way, {delay_ms} ms one-way, drop every {drop_every} s");
    let b2 = bytes.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!("{{\"event\":\"exit\",\"bytes\":{}}}", b2.load(Ordering::Relaxed));
        std::process::exit(0);
    });
    loop {
        let (c, _) = l.accept().await?;
        let to = to.clone();
        let (k1, k2) = (kill_tx.subscribe(), kill_tx.subscribe());
        let (b1, b2) = (bytes.clone(), bytes.clone());
        tokio::spawn(async move {
            let Ok(s) = TcpStream::connect(&to).await else { return };
            let _ = c.set_nodelay(true);
            let _ = s.set_nodelay(true);
            let (cr, cw) = c.into_split();
            let (sr, sw) = s.into_split();
            tokio::join!(pipe(cr, sw, shape, b1, k1), pipe(sr, cw, shape, b2, k2));
        });
    }
}
