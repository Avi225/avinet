#![allow(dead_code)] // Rust compiler does not know main_wasm gets consumed and throws dead code warnings left and right
use eframe::egui;
use shared::{ClientMessage, ServerMessage};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use futures_util::sink::SinkExt as _;
use futures_util::stream::StreamExt;
use gloo_net::websocket::{Message as WsMessage, futures::WebSocket};
use wasm_bindgen_futures::spawn_local;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

struct WsConnection
{
	tx: futures_channel::mpsc::UnboundedSender<WsMessage>,
}

struct SharedAsyncState
{
	client_id: Option<Uuid>,
	connection_status: String,
	chat_state: Option<shared::ChatState>,
}

impl Default for SharedAsyncState
{
	fn default() -> Self
	{
		Self
		{
			client_id: None,
			connection_status: "Disconnected".to_string(),
			chat_state: None,
		}
	}
}

struct MyApp
{
	send_nick: String,
	send_msg: String,

	ws_sender: Option<futures_channel::mpsc::UnboundedSender<WsMessage>>,

	client_id_display: Option<Uuid>,
	connection_status_display: String,
	chat_state_display: Option<shared::ChatState>,

	shared_state: Arc<Mutex<SharedAsyncState>>,
}


impl Default for MyApp
{
	fn default() -> Self
	{
		let shared_async_state = SharedAsyncState::default();
		Self
		{
			send_nick: "john".to_string(),
			send_msg: "hello".to_string(),

			ws_sender: None,

			client_id_display: shared_async_state.client_id,
			connection_status_display: shared_async_state.connection_status.clone(),
			chat_state_display: shared_async_state.chat_state.clone(),

			shared_state: Arc::new(Mutex::new(shared_async_state)),
		}
	}
}

impl MyApp
{
	fn connect(&mut self, cc: &eframe::CreationContext)
	{
		{
			let mut state_guard = self.shared_state.lock().unwrap();
			state_guard.connection_status = "Connecting...".to_string();
		}
		let ui_update_ctx = cc.egui_ctx.clone();

		let shared_state_for_tasks = self.shared_state.clone();

		let window = web_sys::window().expect("should have a Window");
		let location = window.location();
		let host = location.host().expect("should have a host");
		let protocol = location.protocol().expect("should have a protocol");

		let ws_protocol = if protocol == "https:" { "wss:" } else { "ws:" };
		let ws_url = format!("{}//{}/ws", ws_protocol, host);

		log::info!("Connecting to WebSocket at: {}", ws_url);
		let ws_result = WebSocket::open(&ws_url);

		match ws_result
		{
			Ok(ws_socket) =>
			{
				let (mut ws_write_sink, mut ws_read_stream) = ws_socket.split();

				let (ui_tx_to_ws_task, mut ui_rx_for_ws_task) = futures_channel::mpsc::unbounded::<WsMessage>();

				self.ws_sender = Some(ui_tx_to_ws_task);

				{
					let mut state_guard = shared_state_for_tasks.lock().unwrap();
					state_guard.connection_status = "Connection established, tasks starting...".to_string();
				}

				ui_update_ctx.request_repaint();

				let shared_state_for_sender = self.shared_state.clone();
				let ui_update_ctx_for_sender = cc.egui_ctx.clone();

				// Sender task
				spawn_local(async move {
					log::info!("WS Sender task started.");
					while let Some(msg_to_send) = ui_rx_for_ws_task.next().await
					{
						if ws_write_sink.send(msg_to_send).await.is_err()
						{
							log::error!("Error sending message via WebSocket in sender task.");
							break;
						}
					}
					log::info!("WS Sender task finished");
					let mut state_guard = shared_state_for_sender.lock().unwrap();
					if !state_guard.connection_status.starts_with("Error:") && !state_guard.connection_status.starts_with("Failed to connect")
					{
						state_guard.connection_status = "Disconnected (Sender Stopped)".to_string();
					}
					drop(state_guard);
					ui_update_ctx_for_sender.request_repaint();
				});

				// Receiver task

				let shared_state_for_receiver = self.shared_state.clone();
				let ui_update_ctx_for_receiver = cc.egui_ctx.clone();

				spawn_local(async move
				{
					log::info!("WebSocket receiver task started.");
					{
						let mut state_guard = shared_state_for_receiver.lock().unwrap();
						state_guard.connection_status = "Connected (Receiver Active)".to_string();
					}

					ui_update_ctx_for_receiver.request_repaint(); // Request repaint after status update

					while let Some(msg_result) = ws_read_stream.next().await
					{
						let mut state_guard = shared_state_for_receiver.lock().unwrap();
						match msg_result {
							Ok(WsMessage::Text(text)) =>
							{
								log::info!("Received text: {}", text);

								match serde_json::from_str::<ServerMessage>(&text)
								{
									Ok(ServerMessage::Pong) =>
									{
										log::info!("Pong received!");
									}
									Ok(ServerMessage::AssignedId(id)) =>
									{
										log::info!("Received AssignedId: {}", id);
										state_guard.client_id = Some(id);
									}
									Ok(ServerMessage::NewChatState(state)) =>
									{
										log::info!("NewChatState received: {:?}", state);
										state_guard.chat_state = Some(state);
									}
									Ok(ServerMessage::Error(err_msg)) =>
									{
										log::warn!("Server error: {}", err_msg);
									}
									Err(e) =>
									{
										log::warn!("Failed to deserialize ServerMessage from text: {}. Raw: {}", e, text);
									}
								}
							}
							Ok(WsMessage::Bytes(_bin)) =>
							{}
							Err(e) =>
							{
								log::error!("WebSocket error in receiver task: {:?}", e);
								state_guard.connection_status = format!("Error: {:?}", e);
								break;
							}
						}

						drop(state_guard);
						ui_update_ctx_for_receiver.request_repaint();
					}
					log::info!("WebSocket receiver task finished.");
					{
						let mut state_guard = shared_state_for_receiver.lock().unwrap();
						if !state_guard.connection_status.starts_with("Error:") && !state_guard.connection_status.starts_with("Failed to connect")
						{
							state_guard.connection_status ="Disconnected (Receiver Stopped)".to_string();
						}
					}
					ui_update_ctx_for_receiver.request_repaint();
				});
			}
			Err(e) =>
			{
				log::error!("Failed to open WebSocket: {:?}", e);
				let mut state_guard = self.shared_state.lock().unwrap();
				state_guard.connection_status = format!("Failed to connect: {:?}", e);
				drop(state_guard);
				ui_update_ctx.request_repaint();
			}
		}
	}

	fn send_ws_message(&mut self, client_msg: ClientMessage)
	{
		if let Some(sender) = &self.ws_sender
		{
			match serde_json::to_string(&client_msg)
			{
				Ok(msg_str) =>
				{
					if sender.unbounded_send(WsMessage::Text(msg_str)).is_ok()
					{
						log::info!("Sent: {:?}", client_msg);
					}else
					{
						log::error!("Failed to send to WS task (channel error)");
						let mut state_guard = self.shared_state.lock().unwrap();
						state_guard.connection_status = "Error: WS send channel closed".to_string();
					}
				}
				Err(e) =>
				{
					log::error!("Serialization error for ClientMessage: {}", e);
				}
			}
		}else
		{
			log::warn!("Not connected, cannot send message.");    
		}
	}
}

impl eframe::App for MyApp
{
	fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame)
	{
		{
			let state_guard = self.shared_state.lock().unwrap();
			self.client_id_display = state_guard.client_id;
			self.connection_status_display = state_guard.connection_status.clone();
			self.chat_state_display = state_guard.chat_state.clone();
		}

		egui::CentralPanel::default().show(ctx, |ui|
		{
			ui.heading("RTS Client");
			ui.separator();

			if let Some(id) = self.client_id_display
			{
				ui.label(format!("My Client ID: {}", id));
			} else
			{
				ui.label("My Client ID: (Waiting for server...)");
			}

			ui.label(format!("Status: {}", self.connection_status_display));

			if self.ws_sender.is_none() && !self.connection_status_display.starts_with("Failed to connect")
			{
				ui.label("Connection initiated at startup. Refresh the page to attempt reconnect.");
			}

			if self.ws_sender.is_some()
			{
				if ui.button("Ping Server").clicked()
				{
					let my_id = self.shared_state.lock().unwrap().client_id;
					self.send_ws_message(ClientMessage::Ping(my_id));
				}

				ui.horizontal(|ui|
				{
					ui.label("nickname:");
					ui.text_edit_singleline(&mut self.send_nick);
				});
				ui.horizontal(|ui|
				{
					ui.label("message:");
					ui.text_edit_singleline(&mut self.send_msg);
				});

				if ui.button("Send").clicked()
				{
					if !self.send_nick.trim().is_empty() && !self.send_msg.trim().is_empty()
					{
						let client_msg = ClientMessage::Message((self.send_nick.clone(), self.send_msg.clone()));
						self.send_ws_message(client_msg);
						self.send_msg.clear();
					}else
					{
						log::warn!("nickname or message is empty, not sending.");   
					}
				}
			}

			ui.separator();

			if let Some(cs) = &self.chat_state_display
			{
				ui.separator();
				ui.label("chat");

				egui::ScrollArea::vertical()
					.max_height(300.0)
					.max_width(300.0)
					.stick_to_bottom(true)
					.show(ui, |ui|
				{
					if cs.messages.is_empty()
					{
						ui.label("it's empty here...");
					}else
					{
						for(nick, msg) in &cs.messages
						{
							ui.add(egui::Label::new(format!("{}: {}", nick, msg)).wrap());
						}
					}
				});
			}
			else
			{
				ui.label("loading...");	
			}
		});
	}
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn main_wasm()
{
	console_error_panic_hook::set_once();
	wasm_logger::init(wasm_logger::Config::default());
	log::info!("WASM App main() starting");

	let web_options = eframe::WebOptions::default();

	struct AppCreator;

	impl AppCreator
	{
		fn new(cc: &eframe::CreationContext<'_>) -> MyApp
		{
			let mut app = MyApp::default();
			app.connect(cc);
			app
		}
	}

	spawn_local(async
	{
		let canvas_element = web_sys::window()
		    .and_then(|win| win.document())
		    .and_then(|doc| doc.get_element_by_id("main_canvas"))
		    .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
		    .expect("Should have a canvas element with id 'main_canvas'");

		eframe::WebRunner::new()
			.start(
				canvas_element,
				web_options,
				Box::new(|cc| Ok(Box::new(AppCreator::new(cc)))),
			)
			.await
			.expect("failed to start eframe");
	});
	log::info!("WASM App main() finished setup, eframe running in spawned task.");
}
