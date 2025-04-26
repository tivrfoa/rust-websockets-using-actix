Prompt:

- Create a chat web application.
- The server should be in Rust, and the communication will use websockets.
- Create a login page for users.
- The frontend should send a header with a token representing the logged in user. This token was created when the user logged in.
- At each request, the server should log the ip and name of the user that is sending the message, as well as the ip and name of the user that is receiving the message.

---

Okay, let's outline the structure and provide code examples for a Rust WebSocket chat application with login, token authentication via headers (using the subprotocol workaround for browser WebSockets), and specific logging requirements.

This example uses `actix-web` for the web server and WebSocket handling, `serde` for JSON serialization, `uuid` for token generation (simplistic, JWT recommended for production), and `tracing` for logging.

**Project Structure:**

```
chat-app/
├── Cargo.toml
├── static/
│   ├── index.html     # Login page
│   ├── chat.html      # Chat page
│   └── chat.js        # Chat page JavaScript
└── src/
    ├── main.rs        # Server setup, routes
    ├── actors.rs      # ChatServer and ChatSession actors
    ├── handlers.rs    # HTTP and WebSocket handlers
    └── models.rs      # Data structures (Login, Message, etc.)
```

**1. `Cargo.toml` (Dependencies):**

```toml
[package]
name = "chat-app"
version = "0.1.0"
edition = "2021"

[dependencies]
actix = "0.13"
actix-web = "4"
actix-web-actors = "4.1" # Provides ws actors support
actix-files = "0.6" # For serving static files
actix-ws = "0.2"  # Separate crate for newer actix-web ws handling

serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
uuid = { version = "1", features = ["v4", "serde"] }
rand = "0.8"
futures-util = { version = "0.3", default-features = false, features = ["sink", "std"] } # For ws stream

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# State Management
dashmap = "5" # Concurrent HashMap, simpler than Arc<Mutex<HashMap>> for this example
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

**2. `src/models.rs` (Data Structures):**

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use actix::Message as ActixMessage;
use std::net::SocketAddr;
use actix::Addr;
use crate::actors::ChatSession; // Assuming actors are in actors.rs

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    // In a real app, use password hashing
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: Uuid,
}

// --- WebSocket Messages ---

/// Message from server to client
#[derive(Serialize, Debug, ActixMessage)]
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
```

**3. `src/actors.rs` (Chat Server and Session Actors):**

```rust
use actix::prelude::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use rand::rngs::ThreadRng;
use rand::Rng;
use std::net::SocketAddr;
use tracing::{info, warn};

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

     fn stopped(&mut self) {
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

```

**4. `src/handlers.rs` (HTTP and WebSocket Handlers):**

```rust
use actix_web::{web, HttpRequest, HttpResponse, Responder, Error};
use actix_ws; // Use the specific ws crate
use actix::Addr;
use dashmap::DashMap;
use uuid::Uuid;
use std::sync::Arc;
use std::net::SocketAddr;
use tracing::{info, warn, instrument};


use crate::models::{LoginRequest, LoginResponse, UserInfo};
use crate::actors::{ChatServer, ChatSession};

type TokenStore = Arc<DashMap<Uuid, String>>; // Token -> Username
type UserStore = Arc<DashMap<String, String>>; // Username -> Password (hashed in real app)

#[instrument(skip(user_store, token_store))]
pub async fn login(
    req: web::Json<LoginRequest>,
    user_store: web::Data<UserStore>, // Injected state
    token_store: web::Data<TokenStore>, // Injected state
) -> impl Responder {
    info!(username = %req.username, "Login attempt");
    // --- Basic hardcoded user check ---
    // Replace with actual database lookup and password hashing/verification
    let expected_password = user_store.get(&req.username);

    if expected_password.map_or(false, |p| *p == req.password) {
        let token = Uuid::new_v4();
        info!(username = %req.username, token = %token, "Login successful, generated token");
        token_store.insert(token, req.username.clone()); // Store token mapping
        HttpResponse::Ok().json(LoginResponse { token })
    } else {
        warn!(username = %req.username, "Login failed: Invalid credentials");
        HttpResponse::Unauthorized().body("Invalid username or password")
    }
}

// WebSocket Upgrade Handler
#[instrument(skip(chat_server, token_store, req, stream))]
pub async fn ws_route(
    req: HttpRequest,
    stream: web::Payload,
    chat_server: web::Data<Addr<ChatServer>>, // Injected ChatServer address
    token_store: web::Data<TokenStore>,       // Injected TokenStore
) -> Result<HttpResponse, Error> {
    let ip_addr = req.peer_addr().unwrap_or_else(|| {
        // Provide a default/dummy address if peer_addr is None
        "0.0.0.0:0".parse::<SocketAddr>().unwrap()
    });
    info!(ip = %ip_addr, "WebSocket connection attempt");

    // --- Token Authentication via Sec-WebSocket-Protocol Header ---
    // Browsers allow setting this header during WebSocket connection.
    let token_str = req.headers().get("Sec-WebSocket-Protocol")
        .and_then(|h| h.to_str().ok())
        .and_then(|protocols| protocols.split(',').map(|s| s.trim()).find(|s| Uuid::parse_str(s).is_ok())); // Find the first valid UUID in the protocol list


    let user_info = match token_str.and_then(|s| Uuid::parse_str(s).ok()) {
        Some(token) => {
            if let Some(username_ref) = token_store.get(&token) {
                 let username = username_ref.value().clone(); // Clone username from DashMap ref
                 info!(ip = %ip_addr, token = %token, username = %username, "Token validated successfully");
                 Some(UserInfo { username, token })
            } else {
                 warn!(ip = %ip_addr, token = %token, "Invalid or expired token received");
                 None
            }
        }
        None => {
            warn!(ip = %ip_addr, "No valid token found in Sec-WebSocket-Protocol header");
            None
        }
    };

    if let Some(user) = user_info {
        // Upgrade connection and start ChatSession actor
         let protocols = token_str.map(|t| vec![t]).unwrap_or_default(); // Echo back the token subprotocol if found

         match actix_ws::start(
             ChatSession::new(0, chat_server.get_ref().clone(), user.username, ip_addr), // ID 0 initially, ChatServer assigns real ID
             &req,
             stream,
         ) {
             Ok(mut response) => {
                  // If we found a token, make sure it's echoed in the response protocol header
                 if let Some(token_protocol) = token_str {
                      response.headers_mut().insert(
                          actix_web::http::header::SEC_WEBSOCKET_PROTOCOL,
                          actix_web::http::header::HeaderValue::from_str(token_protocol).unwrap(),
                      );
                  }
                  info!(ip = %ip_addr, username = %user.username, "WebSocket upgrade successful");
                  Ok(response)
             }
             Err(e) => {
                  error!(ip = %ip_addr, username = %user.username, error = %e, "WebSocket upgrade failed");
                  Err(e.into()) // Convert actix_ws::HandshakeError into actix_web::Error
             }
         }

    } else {
        // Token validation failed - Reject WebSocket upgrade
        error!(ip = %ip_addr, "WebSocket connection rejected due to failed authentication");
        Ok(HttpResponse::Unauthorized().body("Invalid or missing authentication token"))
    }
}
```

**5. `src/main.rs` (Server Setup):**

```rust
use actix::Actor;
use actix_web::{web, App, HttpServer, middleware};
use actix_files::Files;
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;
use tracing::Level;
use tracing_subscriber::EnvFilter;

mod actors;
mod handlers;
mod models;

use actors::ChatServer;

// Simple in-memory storage (Replace with DB in production)
type TokenStore = Arc<DashMap<Uuid, String>>; // Token -> Username
type UserStore = Arc<DashMap<String, String>>; // Username -> Password (hashed in real app)

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize logging
    let filter = EnvFilter::builder()
        .with_default_directive(Level::INFO.into()) // Default log level
        .from_env_lossy(); // Allow overriding with RUST_LOG env var
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE) // Log when spans close
        .init();

    tracing::info!("Starting application");

    // --- Shared State ---
    // Start ChatServer actor
    let chat_server = ChatServer::new().start();
    tracing::info!("ChatServer actor started");

    // Create in-memory user store (add some dummy users)
    let user_store: UserStore = Arc::new(DashMap::new());
    user_store.insert("alice".to_string(), "password123".to_string()); // Never store plain text passwords!
    user_store.insert("bob".to_string(), "securepass".to_string());
    tracing::info!("In-memory user store initialized");

    // Create in-memory token store
    let token_store: TokenStore = Arc::new(DashMap::new());
    tracing::info!("In-memory token store initialized");

    // --- Start HTTP Server ---
    let bind_address = "127.0.0.1:8080";
    tracing::info!("Starting HTTP server at http://{}", bind_address);

    HttpServer::new(move || {
        App::new()
            // --- Middleware ---
            .wrap(middleware::Logger::default()) // Basic request logging
            .wrap(middleware::NormalizePath::trim()) // Trim trailing slashes

            // --- Share State ---
            .app_data(web::Data::new(chat_server.clone())) // Share ChatServer address
            .app_data(web::Data::new(user_store.clone())) // Share UserStore
            .app_data(web::Data::new(token_store.clone())) // Share TokenStore

            // --- API Routes ---
            .route("/api/login", web::post().to(handlers::login))

            // --- WebSocket Route ---
            .route("/ws/", web::get().to(handlers::ws_route)) // Note the trailing slash if needed

            // --- Static Files (Login/Chat HTML/JS) ---
            .service(Files::new("/", "static/").index_file("index.html")) // Serve index.html for login
    })
    .bind(bind_address)?
    .run()
    .await
}
```

**6. `static/index.html` (Login Page):**

```html
<!DOCTYPE html>
<html>
<head>
    <title>Chat Login</title>
    <style>
        body { font-family: sans-serif; display: flex; justify-content: center; align-items: center; min-height: 100vh; background-color: #f4f4f4; }
        .login-container { background: white; padding: 2em; border-radius: 8px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
        label { display: block; margin-bottom: 0.5em; }
        input[type="text"], input[type="password"] { width: 100%; padding: 0.8em; margin-bottom: 1em; border: 1px solid #ccc; border-radius: 4px; box-sizing: border-box; }
        button { background-color: #5cb85c; color: white; padding: 0.8em 1.5em; border: none; border-radius: 4px; cursor: pointer; font-size: 1em; }
        button:hover { background-color: #4cae4c; }
        .error { color: red; margin-top: 1em; }
    </style>
</head>
<body>
    <div class="login-container">
        <h2>Login</h2>
        <form id="loginForm">
            <div>
                <label for="username">Username:</label>
                <input type="text" id="username" name="username" required>
            </div>
            <div>
                <label for="password">Password:</label>
                <input type="password" id="password" name="password" required>
            </div>
            <button type="submit">Login</button>
        </form>
        <div id="errorMessage" class="error"></div>
    </div>

    <script>
        const form = document.getElementById('loginForm');
        const errorMessageDiv = document.getElementById('errorMessage');

        form.addEventListener('submit', async (event) => {
            event.preventDefault(); // Prevent default form submission
            errorMessageDiv.textContent = ''; // Clear previous errors

            const username = document.getElementById('username').value;
            const password = document.getElementById('password').value;

            try {
                const response = await fetch('/api/login', {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                    },
                    body: JSON.stringify({ username, password }),
                });

                if (response.ok) {
                    const data = await response.json();
                    console.log('Login successful, token:', data.token);
                    // Store token (use localStorage for persistence across browser sessions, sessionStorage for single session)
                    localStorage.setItem('chatToken', data.token);
                    // Redirect to chat page
                    window.location.href = '/chat.html'; // Assuming chat.html is served by actix-files
                } else {
                    const errorText = await response.text();
                    console.error('Login failed:', response.status, errorText);
                    errorMessageDiv.textContent = `Login failed: ${errorText || response.statusText}`;
                }
            } catch (error) {
                console.error('Error during login request:', error);
                errorMessageDiv.textContent = 'An error occurred during login. Please try again.';
            }
        });
    </script>
</body>
</html>
```

**7. `static/chat.html` (Chat Page):**

```html
<!DOCTYPE html>
<html>
<head>
    <title>Web Chat</title>
    <style>
        body { font-family: sans-serif; margin: 0; display: flex; flex-direction: column; height: 100vh; }
        #messages { list-style-type: none; margin: 0; padding: 1em; overflow-y: scroll; flex-grow: 1; border-bottom: 1px solid #ccc; background-color: #f9f9f9; }
        #messages li { padding: 0.5em 0; }
        #messages li:nth-child(odd) { background-color: #eee; }
        #form { display: flex; padding: 1em; background: #e0e0e0; }
        #input { border: 1px solid #ccc; padding: 0.8em; flex-grow: 1; margin-right: 0.5em; border-radius: 4px; }
        button { background: #5cb85c; border: none; padding: 0.8em 1.5em; color: white; border-radius: 4px; cursor: pointer; }
        button:hover { background: #4cae4c; }
        .info { font-style: italic; color: gray; }
        .error { color: red; font-weight: bold; }
        .private { color: purple; font-style: italic; }
    </style>
</head>
<body>
    <ul id="messages">
         <li class="info">Connecting...</li>
    </ul>
    <form id="form" action="">
        <input id="input" autocomplete="off" placeholder="Type message or command (/list, /msg user message)"/>
        <button>Send</button>
    </form>

    <script src="chat.js"></script>
</body>
</html>
```

**8. `static/chat.js` (Chat Page JavaScript):**

```javascript
window.addEventListener('load', () => {
    const messages = document.getElementById('messages');
    const form = document.getElementById('form');
    const input = document.getElementById('input');
    const token = localStorage.getItem('chatToken'); // Retrieve token

    function addMessage(msg, type = 'normal') {
        const item = document.createElement('li');
        item.textContent = msg;
        if (type === 'info' || type === 'error' || type === 'private') {
             item.classList.add(type);
        }
        // Simple detection for private messages based on content
        if (msg.startsWith('(Private from') || msg.startsWith('(Message sent to') || msg.startsWith('(User')) {
            item.classList.add('private');
        } else if (msg.includes(' joined') || msg.includes(' left') || msg.startsWith('Welcome') || msg.startsWith('Online users:')) {
            item.classList.add('info');
        }
        messages.appendChild(item);
        messages.scrollTop = messages.scrollHeight; // Scroll to bottom
    }

    if (!token) {
        addMessage('Authentication token not found. Redirecting to login...', 'error');
        setTimeout(() => {
            window.location.href = '/'; // Redirect to login page (index.html)
        }, 2000);
        return; // Stop execution
    }

    // Construct WebSocket URL
    const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${wsProtocol}//${window.location.host}/ws/`; // Adjust if server runs elsewhere

    // --- Connect WebSocket with Token in Subprotocol ---
    let ws;
    try {
         // The browser sends the token in the Sec-WebSocket-Protocol header
        ws = new WebSocket(wsUrl, token); // Pass token as the subprotocol string
        addMessage('Connecting to chat server...', 'info');
    } catch (e) {
        addMessage(`Failed to create WebSocket: ${e}`, 'error');
         console.error("WebSocket creation error:", e);
        return;
    }


    ws.onopen = (event) => {
         console.log("WebSocket connection opened:", event);
        // Find and remove the initial "Connecting..." message
         const connectingMsg = Array.from(messages.children).find(li => li.textContent.includes('Connecting...'));
         if (connectingMsg) connectingMsg.remove();
         addMessage('Connected!', 'info');
        input.focus();
    };

    ws.onmessage = (event) => {
        console.log('Message from server:', event.data);
        addMessage(event.data);
    };

    ws.onerror = (event) => {
        console.error('WebSocket error:', event);
        addMessage('WebSocket error occurred. Connection might be lost.', 'error');
    };

    ws.onclose = (event) => {
        console.log('WebSocket connection closed:', event.code, event.reason);
        let message = `Disconnected: ${event.reason || 'Connection closed'}`;
        if (event.code !== 1000 && event.code !== 1005) { // 1000=Normal, 1005=No Status Rcvd
             message += ` (Code: ${event.code})`;
        }
         addMessage(message, 'error');
         // Optionally disable input or attempt reconnect
         input.disabled = true;
         form.querySelector('button').disabled = true;
    };

    form.addEventListener('submit', (event) => {
        event.preventDefault();
        if (input.value && ws && ws.readyState === WebSocket.OPEN) {
            console.log('Sending:', input.value)
            ws.send(input.value);
            input.value = ''; // Clear input
        } else {
             console.warn("Cannot send message, WebSocket not open or input empty.");
             if (!ws || ws.readyState !== WebSocket.OPEN) {
                 addMessage('Not connected to server.', 'error');
             }
        }
         input.focus();
    });

});

```

**How to Run:**

1.  Make sure you have Rust installed (`rustup`).
2.  Create the directory structure and save the files.
3.  Navigate to the `chat-app` directory in your terminal.
4.  Build the project: `cargo build`
5.  Run the server: `cargo run` (or set `RUST_LOG=trace` for more detailed logs: `RUST_LOG=trace cargo run`)
6.  Open your web browser and go to `http://127.0.0.1:8080`.
7.  Log in with `alice`/`password123` or `bob`/`securepass`.
8.  You'll be redirected to `chat.html`, and the WebSocket connection will be established.
9.  Open another browser tab/window, go to the same address, and log in as the *other* user.
10. Send messages between the two windows. Check the server console logs to see the sender/receiver IP/name logging for each message.

**Key Points & Improvements:**

* **Authentication:** The token is passed via the `Sec-WebSocket-Protocol` header during the WebSocket handshake. The server validates this token before upgrading the connection. This is a common workaround for the browser `WebSocket` API's limitation in setting arbitrary headers directly for the initial upgrade request.
* **Logging:** The `ChatServer`'s `handle` method for `ClientMessage` now contains `tracing::info!` calls that log the sender's details (ID, name, IP) and the intended recipient (either "all" for broadcast or the specific recipient's details for private messages).
* **Error Handling:** Basic error handling is included, but more robust handling (e.g., specific error messages, reconnection logic on the client) is needed for production.
* **Security:**
    * **HTTPS/WSS:** Absolutely essential for production. Use a reverse proxy like Nginx or Caddy, or configure Actix directly with TLS.
    * **Password Hashing:** Store hashed passwords (e.g., using `argon2` or `bcrypt` crates) instead of plain text.
    * **Input Validation:** Sanitize and validate all inputs on the server.
    * **JWT:** Use JSON Web Tokens instead of simple UUIDs for better security (expiry, claims). Crates like `jsonwebtoken` can be used.
* **State Management:** `DashMap` is used for thread-safe in-memory storage. For persistence and scalability, use a database (like PostgreSQL with `sqlx` or `diesel`) and potentially a message queue (Redis Pub/Sub, Kafka) for inter-instance communication if scaling horizontally.
* **Message Format:** Using JSON for messages (`{"type": "chat", "content": "...", "to": "user"}`) would be more structured than plain text.
* **Frontend:** The frontend is very basic. A framework like React, Vue, or Svelte would provide a much better structure for a real application.
