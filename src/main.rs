use clap::Parser;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = 8080)]
    port: u16,
    #[arg(short, long, value_delimiter = ' ', num_args = 1..)]
    peers: Vec<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct TimeMessage {
    node_id: String,
    timestamp: u128,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let addr = format!("127.0.0.1:{}", args.port);
    let node_id = addr.clone();

    let listener = TcpListener::bind(&addr).await?;
    println!("LLTS Node listening on {}", addr);

    let peers = Arc::new(Mutex::new(HashMap::new()));
    let time_offset = Arc::new(Mutex::new(0i128));

    for peer_addr in args.peers {
        match TcpStream::connect(&peer_addr).await {
            Ok(stream) => {
                let mut peers_map = peers.lock().await;
                peers_map.insert(peer_addr.clone(), stream);
                println!("Connected to peer: {}", peer_addr);
            }
            Err(e) => {
                eprintln!("Failed to connect to peer {}: {}", peer_addr, e);
            }
        }
    }

    let peers_clone = Arc::clone(&peers);
    let time_offset_clone = Arc::clone(&time_offset);
    let node_id_clone = node_id.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let current_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i128;
            let adjusted_time = current_time + *time_offset_clone.lock().await;
            let message = TimeMessage {
                node_id: node_id_clone.clone(),
                timestamp: adjusted_time as u128,
            };
            let message_bytes = serde_json::to_vec(&message).unwrap();

            let mut peers_map = peers_clone.lock().await;
            for (peer_id, stream) in peers_map.iter_mut() {
                if let Err(e) = stream.write_all(&message_bytes).await {
                    eprintln!("Failed to send time message to peer {}: {}", peer_id, e);
                }
            }
        }
    });

    loop {
        let (socket, addr) = listener.accept().await?;
        let peers_clone = Arc::clone(&peers);
        let time_offset_clone = Arc::clone(&time_offset);

        tokio::spawn(async move {
            handle_peer(socket, addr, peers_clone, time_offset_clone).await;
        });
    }
}

async fn handle_peer(socket: TcpStream, addr: SocketAddr, peers: Arc<Mutex<HashMap<String, TcpStream>>>, time_offset: Arc<Mutex<i128>>) {
    println!("New peer connected: {}", addr);
    let peer_id = addr.to_string();

    {
        let mut peers_map = peers.lock().await;
        peers_map.insert(peer_id.clone(), socket);
    }

    let mut buffer = [0; 1024];
    loop {
        let mut peers_map = peers.lock().await;
        if let Some(socket) = peers_map.get_mut(&peer_id) {
            match socket.read(&mut buffer).await {
                Ok(n) if n == 0 => {
                    println!("Peer {} disconnected", peer_id);
                    peers_map.remove(&peer_id);
                    return;
                }
                Ok(n) => {
                    let message: Result<TimeMessage, _> = serde_json::from_slice(&buffer[0..n]);
                    if let Ok(msg) = message {
                        println!("Received time message from {}: {:?}", peer_id, msg);
                        let current_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i128;
                        let mut offset = time_offset.lock().await;
                        let time_diff = msg.timestamp as i128 - (current_time + *offset);
                        *offset += time_diff / 2;
                        println!("Adjusted time offset: {}", *offset);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to read from socket for peer {}: {}", peer_id, e);
                    peers_map.remove(&peer_id);
                    return;
                }
            }
        }
    }
}