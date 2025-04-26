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
            .route("/ws", web::get().to(handlers::ws_route)) // Note the trailing slash if needed

            // --- Static Files (Login/Chat HTML/JS) ---
            .service(Files::new("/", "static/").index_file("index.html")) // Serve index.html for login
    })
    .bind(bind_address)?
    .run()
    .await
}
