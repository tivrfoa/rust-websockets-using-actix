use actix::prelude::*;
use actix_web_actors::ws;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use rand::rngs::ThreadRng;
use std::net::SocketAddr;
use tracing::{error, info, trace, warn};

use crate::models::{ServerMessage, ClientMessage, Connect, Disconnect, SessionInfo};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

// --- ChatSession Actor ---

#[derive(Debug)]
pub struct ChatSession {
    id: usize, // Unique session ID
    hb: Instant, // Last heartbeat received
    addr: Addr<ChatServer>, // Address of the ChatServer actor
    username: String,
    ip_addr: SocketAddr,
}

impl ChatSession {
    pub fn new(id: usize, addr: Addr<ChatServer>, username: String, ip_addr: SocketAddr) -> Self {
        Self { id, hb: Instant::now(), addr, username, ip_addr }
    }

    fn hb(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
                warn!(session_id = act.id, username = %act.username, ip = %act.ip_addr, "WebSocket client timed out. Disconnecting.");
                act.addr.do_send(Disconnect { id: act.id, username: act.username.clone() });
                ctx.stop();
                return;
            }
            ctx.ping(b"");
        });
    }
}

impl Actor for ChatSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!(session_id = self.id, username = %self.username, ip = %self.ip_addr, "ChatSession started");
        self.hb(ctx); // Start heartbeat process

        // Register self with ChatServer
        let addr = ctx.address();
        self.addr
            .send(Connect {
                addr, // Send own address to ChatServer
                username: self.username.clone(),
                ip_addr: self.ip_addr,
            })
            .into_actor(self)
            .then(|res, act, ctx| {
                match res {
                    Ok(session_id) => {
                        act.id = session_id; // Update session ID assigned by ChatServer
                        info!(session_id = act.id, username=%act.username, "Successfully connected to ChatServer");
                    }
                    Err(e) => {
                        error!(session_id = act.id, username=%act.username, error = %e, "Failed to connect to ChatServer");
                        ctx.stop();
                    }
                }
                fut::ready(())
            })
            .wait(ctx);
    }

    fn stopping(&mut self, _: &mut Self::Context) -> Running {
        info!(session_id = self.id, username = %self.username, ip = %self.ip_addr, "ChatSession stopping");
        // Notify ChatServer about disconnection
        self.addr.do_send(Disconnect { id: self.id, username: self.username.clone() });
        Running::Stop
    }
}

/// Handle messages from ChatServer, sending them to the WebSocket client
impl Handler<ServerMessage> for ChatSession {
    type Result = ();

    fn handle(&mut self, msg: ServerMessage, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

/// WebSocket message handler
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ChatSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.hb = Instant::now();
            }
            Ok(ws::Message::Text(text)) => {
                 info!(session_id = self.id, username=%self.username, ip=%self.ip_addr, "Received text message");
                // Send message to ChatServer
                self.addr.do_send(ClientMessage {
                    sender_id: self.id,
                    sender_name: self.username.clone(),
                    sender_ip: self.ip_addr,
                    content: text.to_string(),
                });
            }
            Ok(ws::Message::Binary(bin)) => {
                 warn!(session_id = self.id, username=%self.username, "Received unexpected binary message");
                 ctx.binary(bin); // Echo binary back? Or ignore.
            }
            Ok(ws::Message::Close(reason)) => {
                info!(session_id = self.id, username = %self.username, ?reason, "WebSocket closed by client");
                ctx.close(reason);
                ctx.stop();
            }
            Ok(ws::Message::Continuation(_)) => {
                 warn!(session_id = self.id, username=%self.username, "Continuation frames not supported");
                ctx.stop();
            }
            Ok(ws::Message::Nop) => (),
            Err(e) => {
                error!(session_id = self.id, username = %self.username, error = %e, "WebSocket error occurred");
                // Attempt to notify ChatServer before stopping
                self.addr.do_send(Disconnect { id: self.id, username: self.username.clone() });
                ctx.stop();
            }
        }
    }
}


// --- ChatServer Actor ---

#[derive(Debug)]
pub struct ChatServer {
    sessions: HashMap<usize, SessionInfo>, // Map session ID to session info
    rng: ThreadRng,
    next_id: usize,
}

impl ChatServer {
    pub fn new() -> Self {
        info!("ChatServer starting");
        ChatServer {
            sessions: HashMap::new(),
            rng: rand::thread_rng(),
            next_id: 0, // Simple ID generation
        }
    }

    // Send message to all connected clients
    fn broadcast(&self, message: &str, skip_id: Option<usize>) {
        let server_msg = ServerMessage(message.to_owned());
        for (id, session_info) in &self.sessions {
            if skip_id.map_or(true, |skip| *id != skip) {
                 trace!(target_session_id = id, target_username = %session_info.username, "Broadcasting message");
                if let Err(e) = session_info.addr.try_send(server_msg.clone()) {
                     warn!(target_session_id = id, target_username=%session_info.username, error=%e, "Failed to send message to session");
                    // Consider removing unresponsive clients here or relying on heartbeat
                }
            }
        }
    }

     // Send message to a specific client by username
     fn send_private(&self, recipient_name: &str, message: &str) {
        let server_msg = ServerMessage(message.to_owned());
        let mut found = false;
        for session_info in self.sessions.values() {
            if session_info.username == recipient_name {
                trace!(target_session_id = session_info.id, target_username = %session_info.username, "Sending private message");
                 if let Err(e) = session_info.addr.try_send(server_msg.clone()) {
                     warn!(target_session_id = session_info.id, target_username=%session_info.username, error=%e, "Failed to send private message to session");
                 }
                found = true;
                break; // Assuming unique usernames per session for now
            }
        }
         if !found {
             warn!(recipient=%recipient_name, "Attempted to send private message to unknown or disconnected user");
             // Optionally notify the sender that the recipient wasn't found
         }
    }
}

impl Actor for ChatServer {
    type Context = Context<Self>;

     fn stopped(&mut self, ctx: &mut <Self as actix::Actor>::Context) {
         info!("ChatServer stopped");
     }
}

impl Handler<Connect> for ChatServer {
    type Result = MessageResult<Connect>;

    fn handle(&mut self, msg: Connect, _: &mut Context<Self>) -> Self::Result {
        let id = self.next_id;
        self.next_id += 1;

        let session_info = SessionInfo {
            id,
            username: msg.username.clone(),
            ip_addr: msg.ip_addr,
            addr: msg.addr.clone(),
        };

        info!(session_id = id, username = %msg.username, ip = %msg.ip_addr, "New client connected");

        // Store the new session
        self.sessions.insert(id, session_info);

        // Broadcast join message
        let join_msg = format!("{} ({}) joined", msg.username, msg.ip_addr.ip());
        self.broadcast(&join_msg, Some(id)); // Don't send to the user who just joined

        // Send welcome message to new user
        if let Err(e) = msg.addr.try_send(ServerMessage(format!("Welcome {}!", msg.username))) {
             warn!(session_id = id, username=%msg.username, error=%e, "Failed to send welcome message to new session");
        }

        // Return the assigned session ID
        MessageResult(id)
    }
}

impl Handler<Disconnect> for ChatServer {
    type Result = ();

    fn handle(&mut self, msg: Disconnect, _: &mut Context<Self>) {
         info!(session_id = msg.id, username=%msg.username, "Client disconnected");

        // Remove session
        if let Some(removed_session) = self.sessions.remove(&msg.id) {
            // Broadcast leave message
             let leave_msg = format!("{} ({}) left", removed_session.username, removed_session.ip_addr.ip());
             self.broadcast(&leave_msg, None); // Send to everyone remaining
        } else {
             warn!(session_id = msg.id, username=%msg.username, "Attempted to disconnect non-existent session");
        }
    }
}

impl Handler<ClientMessage> for ChatServer {
    type Result = ();

    fn handle(&mut self, msg: ClientMessage, _: &mut Context<Self>) {
        // **Logging Requirement:** Log sender IP/Name and intended receiver IP/Name
        // For broadcast, receiver is "all". For private messages, find the specific receiver.
        // In this simple broadcast example, we log the sender and indicate broadcast.
        // If implementing private messages, you'd parse `msg.content` for a recipient
        // and look them up in `self.sessions`.

        let formatted_message = format!("{}: {}", msg.sender_name, msg.content);

        // Example: Simple command parsing (extend as needed)
        if msg.content.starts_with("/list") {
             let users: Vec<String> = self.sessions.values().map(|s| format!("{} ({})", s.username, s.ip_addr.ip())).collect();
             let user_list_msg = format!("Online users: {}", users.join(", "));
             // Send only back to the sender
             if let Some(sender_session) = self.sessions.get(&msg.sender_id) {
                 info!(sender_session_id = msg.sender_id, sender_name = %msg.sender_name, sender_ip = %msg.sender_ip, command="/list", "Processing list command");
                 if let Err(e) = sender_session.addr.try_send(ServerMessage(user_list_msg)) {
                     warn!(target_session_id = msg.sender_id, target_username=%msg.sender_name, error=%e, "Failed to send user list");
                 }
             } else {
                 warn!(sender_session_id = msg.sender_id, "Sender session not found for /list command response");
             }

        } else if msg.content.starts_with("/msg") {
            // Example private message: /msg <username> <message>
            let parts: Vec<&str> = msg.content.splitn(3, ' ').collect();
            if parts.len() == 3 {
                let recipient_name = parts[1];
                let private_content = parts[2];
                let private_formatted_message = format!("(Private from {}) {}", msg.sender_name, private_content);

                // Log Sender and Receiver Info
                 if let Some(recipient_session) = self.sessions.values().find(|s| s.username == recipient_name) {
                     info!(
                         sender_session_id = msg.sender_id,
                         sender_name = %msg.sender_name,
                         sender_ip = %msg.sender_ip,
                         recipient_session_id = recipient_session.id,
                         recipient_name = %recipient_session.username,
                         recipient_ip = %recipient_session.ip_addr,
                         message_type = "private",
                         "Processing private message"
                     );
                     self.send_private(recipient_name, &private_formatted_message);
                      // Optionally send confirmation back to sender
                      if let Some(sender_session) = self.sessions.get(&msg.sender_id) {
                          sender_session.addr.do_send(ServerMessage(format!("(Message sent to {})", recipient_name)));
                      }

                 } else {
                     warn!(
                         sender_session_id = msg.sender_id,
                         sender_name = %msg.sender_name,
                         sender_ip = %msg.sender_ip,
                         intended_recipient = recipient_name,
                         message_type = "private",
                         "Recipient not found for private message"
                     );
                      // Notify sender that the user is offline/unknown
                      if let Some(sender_session) = self.sessions.get(&msg.sender_id) {
                          sender_session.addr.do_send(ServerMessage(format!("(User '{}' not found or offline)", recipient_name)));
                      }
                 }

            } else {
                 // Invalid private message format, notify sender
                 if let Some(sender_session) = self.sessions.get(&msg.sender_id) {
                     sender_session.addr.do_send(ServerMessage("(Invalid private message format. Use: /msg <username> <message>)".to_string()));
                 }
            }

        } else {
             // Broadcast message
             info!(
                 sender_session_id = msg.sender_id,
                 sender_name = %msg.sender_name,
                 sender_ip = %msg.sender_ip,
                 recipient = "all", // Indicating broadcast
                 message_type = "broadcast",
                 "Processing broadcast message"
             );
             self.broadcast(&formatted_message, Some(msg.sender_id)); // Broadcast to others
        }
    }
}
