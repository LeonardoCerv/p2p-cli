use std::{collections::HashMap, fmt, str::FromStr, fs};

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures_lite::StreamExt;
use iroh::{Endpoint, NodeAddr, NodeId, Watcher};
use iroh_gossip::{
    api::{Event, GossipReceiver, GossipSender},
    net::{Gossip, GOSSIP_ALPN},
    proto::TopicId,
};
use serde::{Deserialize, Serialize};

#[cfg(target_os = "windows")]
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED};

#[cfg(windows)]
use colored::control;

mod camera;
mod display;

use camera::CameraCapture;
use display::TerminalDisplay;

#[derive(Parser)]
#[command(name = "p2p-videochat", about = "peer-to-peer video chat app using Iroh")]
struct Cli {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Open,
    Join { ticket: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    body: MessageBody,
    nonce: [u8; 16],
}

#[derive(Debug, Serialize, Deserialize)]
enum MessageBody {
    AboutMe { from: NodeId },
    VideoFrame { 
        from: NodeId, 
        frame_data: Vec<u8>,
        width: u32,
        height: u32,
    },
    RoomFull { from: NodeId, target: NodeId },
    KeepAlive { from: NodeId },
}

impl Message {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(Into::into)
    }

    fn new(body: MessageBody) -> Self {
        Self {
            body,
            nonce: rand::random(),
        }
    }

    fn to_vec(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("Serialization should never fail")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompactNodeInfo {
    node_id: NodeId,
    direct_addresses: Vec<std::net::SocketAddr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ticket {
    topic: TopicId,
    nodes: Vec<CompactNodeInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TicketRegistry {
    tickets: HashMap<String, Ticket>,
}

impl TicketRegistry {
    fn load_or_create() -> Self {
        let path = dirs::home_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap())
            .join(".p2p-video-chat-tickets.json");
        
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(registry) = serde_json::from_str(&content) {
                return registry;
            }
        }
        
        Self { tickets: HashMap::new() }
    }
    
    fn save(&self) -> Result<()> {
        let path = dirs::home_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap())
            .join(".p2p-video-chat-tickets.json");
        
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
    
    fn generate_short_code(&self) -> String {
        let chars = b"0123456789abcdefghijklmnopqrstuvwxyz";
        loop {
            let code: String = (0..8)
                .map(|_| chars[rand::random::<usize>() % chars.len()] as char)
                .collect();
            
            if !self.tickets.contains_key(&code) {
                return code;
            }
        }
    }
    
    fn register_ticket(&mut self, ticket: Ticket) -> Result<String> {
        let code = self.generate_short_code();
        self.tickets.insert(code.clone(), ticket);
        self.save()?;
        Ok(code)
    }
    
    fn get_ticket(&self, code: &str) -> Option<&Ticket> {
        self.tickets.get(code)
    }
}

impl Ticket {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        postcard::from_bytes(bytes).map_err(Into::into)
    }

    fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("Serialization should never fail")
    }

    fn to_short_code(&self) -> Result<String> {
        let mut registry = TicketRegistry::load_or_create();
        registry.register_ticket(self.clone())
    }
    
    fn from_code_or_full(input: &str) -> Result<Self> {
        if input.len() <= 8 {
            if let Some(ticket) = TicketRegistry::load_or_create().get_ticket(input) {
                return Ok(ticket.clone());
            }
        }
        input.parse()
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", data_encoding::BASE64URL_NOPAD.encode(&self.to_bytes()))
    }
}

impl FromStr for Ticket {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = data_encoding::BASE64URL_NOPAD.decode(s.as_bytes())?;
        Self::from_bytes(&bytes)
    }
}

fn frames_differ(frame1: &[u8], frame2: &[u8], threshold_percent: u8) -> bool {
    if frame1.len() != frame2.len() || frame1.is_empty() {
        return true;
    }
    
    let total_pixels = frame1.len() / 3;
    
    let step = if total_pixels < 1000 { 
        3 
    } else if total_pixels < 10000 { 
        9 
    } else { 
        15 
    };
    
    let mut different_pixels = 0;
    let mut sampled_pixels = 0;
    
    let max_allowed_diff = (total_pixels * threshold_percent as usize) / (100 * (step / 3));
    
    for i in (0..frame1.len() - 2).step_by(step) {
        sampled_pixels += 1;
        
        let pixel_diff = ((frame1[i] as u16).abs_diff(frame2[i] as u16)) +
                        ((frame1[i + 1] as u16).abs_diff(frame2[i + 1] as u16)) +
                        ((frame1[i + 2] as u16).abs_diff(frame2[i + 2] as u16));
        
        if pixel_diff > 45 {
            different_pixels += 1;
            
            if different_pixels > max_allowed_diff {
                return true;
            }
        }
    }
    
    let change_percent = if sampled_pixels > 0 {
        (different_pixels * 100) / sampled_pixels
    } else {
        100
    };
    
    change_percent > threshold_percent as usize
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize colored crate for Windows support
    #[cfg(windows)]
    let _ = control::set_virtual_terminal(true);
    
    let cli = Cli::parse();
    let endpoint = Endpoint::builder().discovery_n0().bind().await?;

    let gossip = Gossip::builder()
        .max_message_size(10 * 1024 * 1024) 
        .spawn(endpoint.clone());
    let _router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(GOSSIP_ALPN, gossip.clone())
        .spawn();

    let (topic_id, node_ids) = match cli.commands {
        Commands::Open => (TopicId::from_bytes(rand::random()), Vec::new()),
        Commands::Join { ticket } => {
            let ticket = Ticket::from_code_or_full(&ticket)?;
            
            if let Some(first_node) = ticket.nodes.first() {
                endpoint.add_node_addr(NodeAddr::new(first_node.node_id)
                    .with_direct_addresses(first_node.direct_addresses.clone()))?;
                (ticket.topic, vec![first_node.node_id])
            } else {
                return Err(anyhow::anyhow!("Invalid ticket: no nodes found"));
            }
        }
    };

    let ticket = {
        let me = endpoint.node_addr().initialized().await;
        Ticket {
            topic: topic_id,
            nodes: vec![CompactNodeInfo {
                node_id: me.node_id,
                direct_addresses: me.direct_addresses.into_iter().collect(),
            }],
        }
    };
    
    println!("> room code: {}", ticket.to_short_code()?);
    println!("> {}... (max 2 people per room)", if node_ids.is_empty() {
        "waiting for peer"
    } else {
        "connecting to peer"
    });
    
    let (sender, receiver) = gossip
        .subscribe_and_join(topic_id, node_ids)
        .await?
        .split();
    println!("> connected!");

    // Initialize camera with Windows COM workaround
    println!("> initializing camera...");
    
    #[cfg(target_os = "windows")]
    {
        unsafe {
            CoUninitialize();
            std::thread::sleep(std::time::Duration::from_millis(100));
        
            let hr = CoInitializeEx(
                None,
                COINIT_APARTMENTTHREADED
            );
            
            if hr.is_err() && hr.0 != 1 {
                eprintln!("Warning: Could not set apartment threading, trying multithreaded: {:?}", hr);
                
                CoUninitialize();
                std::thread::sleep(std::time::Duration::from_millis(50));
                
                let hr2 = CoInitializeEx(
                    None,
                    COINIT_MULTITHREADED
                );
                
                if hr2.is_err() && hr2.0 != 1 {
                    eprintln!("Warning: Could not initialize COM at all: {:?}", hr2);
                }
            }
        }
    }
    
    let mut camera = match CameraCapture::new() {
        Ok(cam) => {
            Some(cam)
        },
        Err(e) => {
            #[cfg(target_os = "windows")]
            {
                println!("> warning: failed to initialize camera: {}", e);
                println!("> this is often caused by Windows Media Foundation issues");
                println!("> troubleshooting steps:");
                println!(">   1. ensure no other applications are using the camera");
                println!(">   2. try running as administrator");
                println!(">   3. check camera permissions in windows privacy settings");
                println!(">   4. restart the application");
                println!("> will send placeholder frames and can still receive video from peers");
            }
            #[cfg(not(target_os = "windows"))]
            {
                println!("> warning: failed to initialize camera: {}", e);
                println!("> will send placeholder frames and can still receive video from peers");
            }
            None
        }
    };

    let mut display: Option<TerminalDisplay> = None;

    sender.broadcast(Message::new(MessageBody::AboutMe {
        from: endpoint.node_id(),
    }).to_vec().into()).await?;

    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel::<(Vec<u8>, u32, u32)>();
    
    let sender_clone = sender.clone();
    let my_id = endpoint.node_id();
    tokio::spawn(subscribe_loop(receiver, sender_clone.clone(), my_id, frame_tx));

    let keepalive_sender = sender.clone();
    let keepalive_id = my_id;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let _ = keepalive_sender.broadcast(Message::new(MessageBody::KeepAlive {
                from: keepalive_id,
            }).to_vec().into()).await;
        }
    });

    let mut interval = tokio::time::interval(std::time::Duration::from_millis(33));
    let mut last_frame: Option<Vec<u8>> = None;
    
    let create_error_frame = || {
        let width = 640u32;
        let height = 480u32;
        let mut frame_data = Vec::with_capacity((width * height * 3) as usize);
        
        let center_x = width / 2;
        let center_y = height / 2;
        
        for y in 0..height {
            for x in 0..width {
                let dx = (x as i32 - center_x as i32).abs();
                let dy = (y as i32 - center_y as i32).abs();
                let dist = ((dx * dx + dy * dy) as f64).sqrt();
                
                if dist < 50.0 {
                    frame_data.extend_from_slice(&[255, 255, 255]);
                } else if (x / 40) % 2 == (y / 40) % 2 {
                    frame_data.extend_from_slice(&[180, 40, 40]);
                } else {
                    frame_data.extend_from_slice(&[120, 20, 20]);
                }
            }
        }
        
        (frame_data, width, height)
    };

    let reduce_frame_size = |frame: &[u8], orig_w: u32, orig_h: u32, new_w: u32, new_h: u32| -> Vec<u8> {
        let mut reduced = Vec::with_capacity((new_w * new_h * 3) as usize);
        
        for y in 0..new_h {
            for x in 0..new_w {
                let orig_x = ((x as f32 / new_w as f32) * orig_w as f32) as u32;
                let orig_y = ((y as f32 / new_h as f32) * orig_h as f32) as u32;
                
                let orig_x = orig_x.min(orig_w - 1);
                let orig_y = orig_y.min(orig_h - 1);
                
                let idx = ((orig_y * orig_w + orig_x) * 3) as usize;
                if idx + 2 < frame.len() {
                    reduced.extend_from_slice(&[frame[idx], frame[idx + 1], frame[idx + 2]]);
                } else {
                    reduced.extend_from_slice(&[0, 0, 0]);
                }
            }
        }
        
        reduced
    };

    let mut frame_counter = 0u32;
    let mut _last_frame_time = std::time::Instant::now();

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Some(ref mut cam) = camera {
                    frame_counter += 1;
                    
                    let should_capture = if cam.is_healthy() {
                        true
                    } else {
                        frame_counter % 2 == 0
                    };
                    
                    if should_capture {
                        let (width, height) = cam.dimensions();
                        match cam.get_frame() {
                            Ok(frame) => {
                                let now = std::time::Instant::now();
                                _last_frame_time = now;
                                
                                if frame.len() >= (width * height * 3) as usize {
                                    let reduced_frame = reduce_frame_size(frame, width, height, 640, 480);

                                    let should_send = if let Some(ref last) = last_frame {
                                        frames_differ(&reduced_frame, last, 1)
                                    } else {
                                        true
                                    };
                                    
                                    if should_send {
                                        let frame_data = reduced_frame.clone();
                                        
                                        let message = Message::new(MessageBody::VideoFrame {
                                            from: endpoint.node_id(),
                                            frame_data,
                                            width: 640,
                                            height: 480,
                                        });
                                        let message_bytes = message.to_vec();
                                        let _ = sender.broadcast(message_bytes.into()).await;
                                        
                                        last_frame = Some(reduced_frame);
                                    }
                                }
                            },
                            Err(e) => {
                                eprintln!("Error capturing frame: {}", e);
                                let (error_frame, error_width, error_height) = create_error_frame();
                                let frame_data = error_frame.clone(); 
                                let message = Message::new(MessageBody::VideoFrame {
                                    from: endpoint.node_id(),
                                    frame_data,
                                    width: error_width,
                                    height: error_height,
                                });
                                let message_bytes = message.to_vec();
                                let _ = sender.broadcast(message_bytes.into()).await;
                            }
                        }
                    }
                } else {
                    let (error_frame, error_width, error_height) = create_error_frame();
                    let frame_data = error_frame.clone();
                    
                    let should_send = if let Some(ref last) = last_frame {
                        frames_differ(&frame_data, last, 5)
                    } else {
                        true
                    };
                    
                    if should_send {
                        let message = Message::new(MessageBody::VideoFrame {
                            from: endpoint.node_id(),
                            frame_data: frame_data.clone(),
                            width: error_width,
                            height: error_height,
                        });
                        let message_bytes = message.to_vec();
                        let _ = sender.broadcast(message_bytes.into()).await;
                        
                        last_frame = Some(frame_data);
                    }
                }
            }
            Some((frame_data, width, height)) = frame_rx.recv() => {
                if display.is_none() {
                    display = Some(TerminalDisplay::new(width, height));
                    println!("> receiving video from peer...");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                
                if let Some(ref mut disp) = display {
                    if let Err(e) = disp.show_frame(&frame_data) {
                        eprintln!("Display error: {}", e);
                    }
                }
            }
        }
    }
}

async fn subscribe_loop(
    mut receiver: GossipReceiver, 
    sender: GossipSender, 
    my_node_id: NodeId,
    frame_tx: tokio::sync::mpsc::UnboundedSender<(Vec<u8>, u32, u32)>
) -> Result<()> {
    let mut connected_peers = std::collections::HashSet::new();
    let mut rejected_peers = std::collections::HashSet::new();
    
    while let Some(event) = receiver.try_next().await? {
        if let Event::Received(msg) = event {
            match Message::from_bytes(&msg.content) {
                Ok(message) => {
                    match message.body {
                MessageBody::AboutMe { from } => {
                    if from == my_node_id {
                        continue;
                    }
                    
                    if rejected_peers.contains(&from) {
                        let _ = sender.broadcast(Message::new(MessageBody::RoomFull {
                            from: my_node_id,
                            target: from,
                        }).to_vec().into()).await;
                        continue;
                    }
                    
                    if connected_peers.len() >= 1 {
                        println!("{} tried to join but room is full. Rejecting connection.", from.fmt_short());
                        rejected_peers.insert(from);
                        for _ in 0..3 {
                            let _ = sender.broadcast(Message::new(MessageBody::RoomFull {
                                from: my_node_id,
                                target: from,
                            }).to_vec().into()).await;
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    } else {
                        connected_peers.insert(from);
                        println!("{} has joined ({}/2 people in room)", from.fmt_short(), connected_peers.len() + 1);
                    }
                },
                MessageBody::VideoFrame { from, frame_data, width, height } => {
                    if from == my_node_id {
                        continue;
                    }
                    
                    if rejected_peers.contains(&from) {
                        let _ = sender.broadcast(Message::new(MessageBody::RoomFull {
                            from: my_node_id,
                            target: from,
                        }).to_vec().into()).await;
                        continue;
                    }
                    
                    let frame_data_raw = frame_data.clone();
                    
                    if connected_peers.contains(&from) {
                        let _ = frame_tx.send((frame_data_raw, width, height));
                    } else if connected_peers.len() < 1 {
                        connected_peers.insert(from);
                        println!("{} has joined ({}/2 people in room)", from.fmt_short(), connected_peers.len() + 1);
                        
                        let _ = frame_tx.send((frame_data_raw, width, height));
                    } else {
                        rejected_peers.insert(from);
                        let _ = sender.broadcast(Message::new(MessageBody::RoomFull {
                            from: my_node_id,
                            target: from,
                        }).to_vec().into()).await;
                    }
                },
                MessageBody::RoomFull { from, target } => {
                    if from != my_node_id && target == my_node_id {
                        println!("Room you tried to join is full. Only 2 people allowed per room.");
                        std::process::exit(1);
                    }
                },
                MessageBody::KeepAlive { from } => {
                    if from == my_node_id {
                        continue;
                    }
                    if !rejected_peers.contains(&from) && connected_peers.len() < 1 {
                        connected_peers.insert(from);
                    }
                }
            }
        },
        Err(e) => {
            eprintln!("Failed to decode message: {}", e);
        }
    }
        }
    }
    Ok(())
}

