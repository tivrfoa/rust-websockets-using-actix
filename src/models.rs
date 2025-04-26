use serde::{Deserialize, Serialize};
use uuid::Uuid;
use actix::Message as ActixMessage;
use std::net::SocketAddr;
use actix::Addr;
use crate::actors::ChatSession; // Assuming actors are in actors.rs

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    // In a real app, use password hashing
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: Uuid,
}

// --- WebSocket Messages ---

/// Message from server to client
#[derive(Clone, Serialize, Debug, ActixMessage)]
#[rtype(result = "()")]
pub struct ServerMessage(pub String);

/// Message from client session actor to chat server actor
#[derive(ActixMessage, Debug)]
#[rtype(result = "()")]
pub struct ClientMessage {
    pub sender_id: usize, // Session ID
    pub sender_name: String,
    pub sender_ip: SocketAddr,
    pub content: String,
    // Add recipient info if implementing private messages
    // pub recipient_name: Option<String>,
}

/// Chat server sends this to connect a new session
#[derive(ActixMessage)]
#[rtype(usize)] // Returns the new session ID
pub struct Connect {
    pub addr: Addr<ChatSession>,
    pub username: String,
    pub ip_addr: SocketAddr,
}

/// Chat server sends this to disconnect a session
#[derive(ActixMessage)]
#[rtype(result = "()")]
pub struct Disconnect {
    pub id: usize,
    pub username: String,
}

// --- User Info ---
#[derive(Debug, Clone)]
pub struct UserInfo {
   pub username: String,
   pub token: Uuid,
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: usize,
    pub username: String,
    pub ip_addr: SocketAddr,
    pub addr: Addr<ChatSession>,
}
