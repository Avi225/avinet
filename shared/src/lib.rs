use serde::{Serialize, Deserialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChatState {
    pub messages: Vec<(String, String)>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ClientMessage {
    Ping(Option<Uuid>),
    Message((String, String)),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMessage {
    Pong,
    AssignedId(Uuid),
    NewChatState(ChatState),
    Error(String),
}
