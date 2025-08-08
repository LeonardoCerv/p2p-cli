use std::{
    collections::HashMap, 
    fmt, 
    str::FromStr, 
    fs, 
    sync::{Arc, Mutex},
    io::{self, Write}
};

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures_lite::StreamExt;
use iroh::{Endpoint, NodeAddr, NodeId, Watcher};
use iroh_gossip::{
    api::{Event, GossipReceiver},
    net::{Gossip, GOSSIP_ALPN},
    proto::TopicId,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "p2p-chat", about = "peer-to-peer chat app using Iroh")]
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
    Message { from: NodeId, text: String },
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
            .join(".p2p-cli-tickets.json");
        
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
            .join(".p2p-cli-tickets.json");
        
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

#[derive(Clone)]
struct TerminalUI {
    messages: Arc<Mutex<Vec<String>>>,
    current_input: Arc<Mutex<String>>,
}

impl TerminalUI {
    fn new() -> Self {
        Self {
            messages: Arc::new(Mutex::new(Vec::new())),
            current_input: Arc::new(Mutex::new(String::new())),
        }
    }

    fn add_message(&self, msg: String) {
        self.messages.lock().unwrap().push(msg);
        self.redraw();
    }

    fn update_input(&self, input: String) {
        *self.current_input.lock().unwrap() = input;
        self.redraw();
    }

    fn redraw(&self) {
        // FIX!! Clear the screen
        print!("\x1B[2J\x1B[1;1H");
        
        for msg in self.messages.lock().unwrap().iter() {
            println!("{}", msg);
        }
        
        print!("> {}", self.current_input.lock().unwrap());
        
        io::stdout().flush().unwrap();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let endpoint = Endpoint::builder().discovery_n0().bind().await?;
    
    let ui = TerminalUI::new();
    //ui.add_message(format!("> our node id: {}", endpoint.node_id()));

    let gossip = Gossip::builder().spawn(endpoint.clone());
    let _router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(GOSSIP_ALPN, gossip.clone())
        .spawn();

    let (topic_id, node_ids) = match cli.commands {
        Commands::Open => (TopicId::from_bytes(rand::random()), Vec::new()),
        Commands::Join { ticket } => {
            let ticket = Ticket::from_code_or_full(&ticket)?;
            //ui.add_message(format!("> joining topic: {}", ticket.topic));
            
            for node in &ticket.nodes {
                endpoint.add_node_addr(NodeAddr::new(node.node_id)
                    .with_direct_addresses(node.direct_addresses.clone()))?;
            }
            
            (ticket.topic, ticket.nodes.iter().map(|n| n.node_id).collect())
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
    
    ui.add_message(format!("Room code! {}", ticket.to_short_code()?));
    //ui.add_message(format!("> full ticket: {}", ticket));
    //ui.add_message("> share either the 8-character code or the full ticket!".to_string());

    ui.add_message(if node_ids.is_empty() {
        "waiting for peers...".to_string()
    } else {
        "connecting to peers...".to_string()
    });
    
    let (sender, receiver) = gossip
        .subscribe_and_join(topic_id, node_ids)
        .await?
        .split();
    ui.add_message("successfully connected!".to_string());
    ui.add_message("-----------------------".to_string());

    sender.broadcast(Message::new(MessageBody::AboutMe {
        from: endpoint.node_id(),
    }).to_vec().into()).await?;

    let ui_clone = ui.clone();
    tokio::spawn(async move {
        subscribe_loop(receiver, ui_clone).await
    });

    let (line_tx, mut line_rx) = mpsc::channel(1);
    let ui_clone = ui.clone();
    std::thread::spawn(move || input_loop(line_tx, ui_clone));

    while let Some(text) = line_rx.recv().await {
        let text = text.trim();
        if !text.is_empty() {
            sender.broadcast(Message::new(MessageBody::Message {
                from: endpoint.node_id(),
                text: text.to_string(),
            }).to_vec().into()).await?;
        }
        ui.add_message(format!("you: {}", text));
    }
    
    Ok(())
}

async fn subscribe_loop(mut receiver: GossipReceiver, ui: TerminalUI) -> Result<()> {
    while let Some(event) = receiver.try_next().await? {
        if let Event::Received(msg) = event {
            match Message::from_bytes(&msg.content)?.body {
                MessageBody::AboutMe { from } => {
                    ui.add_message(format!("{} has joined!", from.fmt_short()));
                }
                MessageBody::Message { from, text } => {
                    ui.add_message(format!("{}: {}", from.fmt_short(), text));
                }
            }
        }
    }
    Ok(())
}

fn input_loop(line_tx: mpsc::Sender<String>, ui: TerminalUI) -> Result<()> {
    let mut buffer = String::new();
    loop {
        std::io::stdin().read_line(&mut buffer)?;
        ui.update_input(buffer.clone());
        line_tx.blocking_send(buffer.clone())?;
        buffer.clear();
        ui.update_input(buffer.clone());
    }
}