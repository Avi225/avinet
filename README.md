# Avinet

A simple server-authoritative template for networked Rust projects, featuring a real-time example chat client. Built with an `axum` server and an `egui` (via `eframe`) WASM client.

**Core:**

*   **Server:**
    *   `axum`: Web framework for handling HTTP and WebSocket connections.
    *   `tokio`: Asynchronous runtime.
    *   `serde`: Serialization/deserialization (JSON for client-server messages).
    *   `uuid`: Generating unique client identifiers.
*   **Client:**
    *   `eframe`/`egui`: GUI and custom rendering.
    *   `gloo-net`: WebSocket communication in WASM.
    *   `futures-util`: Async stream/sink utilities.
    *   `serde`: Serialization/deserialization.
    *   `uuid`: Handling client identifiers.
*   **Shared:**
    *   `serde`: For common message structures.
    *   `uuid`: For shared ID types.

Designed as a starting point for my future networked applications.
