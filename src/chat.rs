use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures_lite::StreamExt;
use iroh::protocol::Router;
use iroh::{Endpoint, NodeAddr, NodeId, Watcher};
use iroh_gossip::{
    api::{Event, GossipReceiver},
    net::{Gossip, GOSSIP_ALPN},
    proto::TopicId,
};
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(name = "p2p-chat")]
#[command(about = "A peer-to-peer chat application using Iroh")]
struct Cli {
    #[arg(short, long, default_value = "anonymous")]
    name: String,
    #[command(subcommand)]
    command: Commands,
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
    AboutMe { from: NodeId, name: String },
    Message { from: NodeId, text: String },
}

impl Message {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(Into::into)
    }

    pub fn new(body: MessageBody) -> Self {
        Self {
            body,
            nonce: rand::random(),
        }
    }

    pub fn to_vec(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serde_json::to_vec is infallible")
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Ticket {
    topic: TopicId,
    nodes: Vec<NodeAddr>,
}

impl Ticket {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(Into::into)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serde_json::to_vec is infallible")
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut text = data_encoding::BASE32_NOPAD.encode(&self.to_bytes()[..]);
        text.make_ascii_lowercase();
        write!(f, "{}", text)
    }
}

impl FromStr for Ticket {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = data_encoding::BASE32_NOPAD.decode(s.to_ascii_uppercase().as_bytes())?;
        Self::from_bytes(&bytes)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let endpoint = Endpoint::builder()
        .discovery_n0()
        .bind()
        .await?;

    println!("> our node id: {}", endpoint.node_id());

    let gossip = Gossip::builder().spawn(endpoint.clone());

    let _router = Router::builder(endpoint.clone())
        .accept(GOSSIP_ALPN, gossip.clone())
        .spawn();

    let (topic_id, node_ids) = match cli.command {
        Commands::Open => {
            let id = TopicId::from_bytes(rand::random());
            let node_ids = vec![];
            (id, node_ids)
        }
        Commands::Join { ticket } => {
            let ticket: Ticket = ticket.parse()?;
            println!("> joining topic: {}", ticket.topic);
            
            for peer in &ticket.nodes {
                endpoint.add_node_addr(peer.clone())?;
            }
            
            let node_ids: Vec<NodeId> = ticket.nodes.iter().map(|addr| addr.node_id).collect();
            (ticket.topic, node_ids)
        }
    };

    let ticket = {
        let me = endpoint.node_addr().initialized().await;
        let nodes = vec![me];
        Ticket {
            topic: topic_id,
            nodes,
        }
    };
    println!("> ticket to join us: {ticket}");

    if node_ids.is_empty() {
        println!("> waiting for peers to join us...");
    } else {
        println!("> trying to connect to {} peers...", node_ids.len());
    }
    
    let (sender, receiver) = if node_ids.is_empty() {
        let topic = gossip.subscribe(topic_id, node_ids).await?;
        topic.split()
    } else {
        gossip.subscribe_and_join(topic_id, node_ids).await?.split()
    };
    println!("> connected!");

    let message = Message::new(MessageBody::AboutMe {
        from: endpoint.node_id(),
        name: cli.name.clone(),
    });

    sender.broadcast(message.to_vec().into()).await?;

    tokio::spawn(subscribe_loop(receiver));

    let (line_tx, mut line_rx) = tokio::sync::mpsc::channel(1);

    std::thread::spawn(move || input_loop(line_tx));


    println!("> type a message and hit enter to broadcast...");

    while let Some(text) = line_rx.recv().await {
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        

        let message = Message::new(MessageBody::Message {
            from: endpoint.node_id(),
            text: text.clone(),
        });

        sender.broadcast(message.to_vec().into()).await?;

        println!("{}: {}", cli.name, text);
    }

    Ok(())
}


async fn subscribe_loop(mut receiver: GossipReceiver) -> Result<()> {
    let mut names = HashMap::new();

    while let Some(event) = receiver.try_next().await? {
        if let Event::Received(msg) = event {
            match Message::from_bytes(&msg.content)?.body {
                MessageBody::AboutMe { from, name } => {
                    names.insert(from, name.clone());
                    println!("> {} is now known as {}", from.fmt_short(), name);
                }
                MessageBody::Message { from, text } => {
                    let name = names
                        .get(&from)
                        .map_or_else(|| from.fmt_short(), String::to_string);
                    println!("{}: {}", name, text);
                }
            }
        }
    }
    Ok(())
}

fn input_loop(line_tx: tokio::sync::mpsc::Sender<String>) -> Result<()> {
    let mut buffer = String::new();
    let stdin = std::io::stdin();
    loop {
        stdin.read_line(&mut buffer)?;
        line_tx.blocking_send(buffer.clone())?;
      
        buffer.clear();
    }
}