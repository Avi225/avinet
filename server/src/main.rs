use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use dotenvy::dotenv;
use shared::{ClientMessage, ServerMessage};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, broadcast};
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

#[derive(Debug)]
struct ConnectedClient
{
    id: Uuid,
}

#[derive(Debug, Clone, Default)]
struct GameState
{
    chat_messages: Vec<(String, String)>
}

struct AppState
{
    clients: Mutex<HashMap<Uuid, ConnectedClient>>,
    game_state: Mutex<GameState>,
    broadcast_tx: broadcast::Sender<ServerMessage>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "server=debug,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let (broadcast_tx, _) = broadcast::channel(100);
    let app_state = Arc::new(AppState {
        clients: Mutex::new(HashMap::new()),
        game_state: Mutex::new(GameState::default()),
        broadcast_tx,
    });

    let client_serve_dir = ServeDir::new("./client/dist")
        .append_index_html_on_directories(true);

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(client_serve_dir)
        .with_state(app_state);

    let port_str = dotenvy::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let port = port_str.parse::<u16>().unwrap_or_else(|e| {
        tracing::warn!(
            "Failed to parse PORT value '{}': {}. Defaulting to 3000.",
            port_str,
            e
        );
        3000
    });
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(&addr).await?;

    tracing::debug!("listening on {}", listener.local_addr()?);
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse
{
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>)
{
    let client_id = Uuid::new_v4();
    {
        let mut clients_guard = state.clients.lock().await;
        clients_guard.insert(client_id, ConnectedClient { id: client_id });
        tracing::info!("[Client {}] Connected! Total clients: {}", client_id, clients_guard.len());
    }

    if socket.send(Message::Text(serde_json::to_string(&ServerMessage::AssignedId(client_id)).unwrap(),)).await.is_err()
    {
        tracing::error!("[Client {}] Failed to send AssingedId. Disconnecting.", client_id);
        state.clients.lock().await.remove(&client_id);
        return;
    }

    {
        let game_state_guard = state.game_state.lock().await;
        let chat_state_for_new_client = shared::ChatState {messages: game_state_guard.chat_messages.clone()};
        if socket.send(Message::Text(serde_json::to_string(&ServerMessage::NewChatState(chat_state_for_new_client)).unwrap())).await.is_err()
        {
            tracing::warn!("[Client {}] Failed to send initial chat state.", client_id);
        }
    }

    let mut broadcast_rx = state.broadcast_tx.subscribe();

    loop
    {
        tokio::select!
        {
            msg_option = socket.recv() =>
            {
                match msg_option {
                    Some(Ok(Message::Text(text))) =>
                    {
                        tracing::debug!("[Client {}] Received text: \"{}\"", client_id, text);
                        match serde_json::from_str::<ClientMessage>(&text)
                        {
                            Ok(ClientMessage::Ping(maybe_known_id)) =>
                            {
                                if let Some(known_id) = maybe_known_id
                                {
                                    tracing::info!("[Client {}] Received Ping (client thinks it's {}). Sending Pong.", client_id, known_id);
                                }else
                                {
                                    tracing::info!("[Client {}] Received Ping (client ID unknown to it). Sending Pong.", client_id);
                                }

                                if socket.send(Message::Text(serde_json::to_string(&ServerMessage::Pong).unwrap())).await.is_err()
                                {
                                    tracing::warn!("[Client {}] Disconnected before pong could be sent.", client_id);
                                    break;
                                }
                            }

                            Ok(ClientMessage::Message((nick, msg))) =>
                            {
                                tracing::info!("[Client {}] Received Message from {}: {}", client_id, nick, msg);
                                let message = (nick, msg);

                                let mut game_state_guard = state.game_state.lock().await;
                                game_state_guard.chat_messages.push(message);

                                if game_state_guard.chat_messages.len() > 100
                                {
                                    game_state_guard.chat_messages.remove(0);
                                }

                                let chat_state_update = shared::ChatState {messages: game_state_guard.chat_messages.clone()};
                                let server_msg = ServerMessage::NewChatState(chat_state_update);
                                drop(game_state_guard);

                                if let Err(e) = state.broadcast_tx.send(server_msg)
                                {
                                    tracing::warn!("[Server] Broadcast error: {}", e);
                                }
                            }
                            Err(e) =>
                            {
                                tracing::warn!("[Client {}] Failed to deserialize client message: {:?}, raw: {}", client_id, e, text);

                                let e = ServerMessage::Error(format!("Invalid message format: {}", e));
                                if socket.send(Message::Text(serde_json::to_string(&e).unwrap())).await.is_err()
                                {
                                    tracing::warn!("[Client {}] Failed to send error message. Disconnecting.", client_id);
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Binary(bin))) =>
                    {
                        tracing::debug!("[Client {}] Received binary (len: {})", client_id, bin.len());
                    }
                    Some(Ok(Message::Ping(_))) =>
                    {
                        tracing::trace!("[Client {}] WebSocket Ping received", client_id);
                    }
                    Some(Ok(Message::Pong(_))) =>
                    {
                        tracing::trace!("[Client {}] WebSocket Pong received", client_id);
                    }
                    Some(Ok(Message::Close(_))) =>
                    {
                        tracing::info!("[Client {}] Sent close frame.", client_id);
                        break;
                    }
                    Some(Err(e)) =>
                    {
                        tracing::error!("[Client {}] WebSocket error: {}", client_id, e);
                        break;
                    }
                    None =>
                    {
                        tracing::info!("[Client {}] Connection closed by peer.", client_id);
                        break;
                    }
                }
            }

            Ok(server_broadcast_msg) = broadcast_rx.recv() =>
            {
                match server_broadcast_msg
                {
                    ServerMessage::NewChatState(_) => 
                    {
                        if socket.send(Message::Text(serde_json::to_string(&server_broadcast_msg).unwrap())).await.is_err()
                        {
                            tracing::warn!("[Client {}] Failed to send broadcast message. Disconnecting.", client_id);
                            break;
                        }
                        tracing::debug!("[Client {}] Sent broadcast message: {:?}", client_id, server_broadcast_msg);
                    }
                    _ =>
                    {

                    }
                }
            }
            
            else =>
            {
                tracing::warn!("[Client {}] Broadcast reciever error or channel closed.", client_id);
            }
        }
    }

    {
        let mut clients_guard = state.clients.lock().await;
        clients_guard.remove(&client_id);
        tracing::info!("[Client {}] Disonnected! Total clients: {}", client_id, clients_guard.len());

    }
}
