use actix_web::{web, HttpRequest, HttpResponse, Responder, Error};
use actix_web_actors::ws;
use actix::Addr;
use dashmap::DashMap;
use uuid::Uuid;
use std::sync::Arc;
use std::net::SocketAddr;
use tracing::{error, info, warn, instrument};


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

         match ws::start(
             ChatSession::new(0, chat_server.get_ref().clone(), user.username.clone(), ip_addr), // ID 0 initially, ChatServer assigns real ID
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
