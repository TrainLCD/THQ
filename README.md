# thq-server

Rust-only telemetry WebSocket server for THQ/TrainLCD.

- WebSocket endpoint: `ws://<host>:<port>/ws`
- Health check: `GET /healthz`
- Broadcasts `location_update` and `log` messages to all subscribers
- Maintains a ring buffer of the latest N events (default 1000)

## Usage

```bash
cargo run -- --host 0.0.0.0 --port 8080
# or with config file
cargo run -- --config config.toml
```

Example `config.toml`:

```toml
host = "0.0.0.0"
port = 8080
ring_size = 1000
```

## Message formats

### subscribe
```json
{"type": "subscribe", "device": "device-id"}
```

### location_update
```json
{
  "id": "uuid",
  "type": "location_update",
  "device": "device-id",
  "state": "arrived | approaching | passing | moving",
  "coords": {
    "latitude": 35.0,
    "longitude": 139.0,
    "accuracy": 5.0,
    "speed": 10.0
  },
  "timestamp": 1234567890
}
```

### log
```json
{
  "id": "uuid",
  "type": "log",
  "device": "device-id",
  "timestamp": 1234567890,
  "log": {
    "type": "system | app | client",
    "level": "debug | info | warn | error",
    "message": "System operational"
  }
}
```

### error
```json
{
  "type": "error",
  "error": {
    "type": "websocket_message_error | json_parse_error | payload_parse_error | accuracy_low | invalid_coords | unknown",
    "reason": "..."
  }
}
```