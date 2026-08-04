extends Node

## Debug-only WebSocket transport.
## Connects to iOS/CLI WebSocket server for development.
## Replaces per-project bridge_ws_client.gd copies.
##
## Must be added to the scene tree (extends Node for _process polling).
## Self-disables in export (non-editor) builds.

signal data_received(json_string: String)

var _ws: WebSocketPeer
var _connected: bool = false
var _reconnect_timer: float = 0.0
var _active: bool = false
var _bridge_mode: String = "optional"
var _attempt_count: int = 0
var _max_attempts: int = 5
var _stopped_retrying: bool = false

const RECONNECT_INTERVAL: float = 3.0
const REQUIRED_MODE_MAX_ATTEMPTS: int = 20
const OPTIONAL_MODE_MAX_ATTEMPTS: int = 2


func configure(bridge_mode: String) -> void:
	_bridge_mode = bridge_mode if bridge_mode in ["optional", "required"] else "optional"
	_max_attempts = REQUIRED_MODE_MAX_ATTEMPTS if _bridge_mode == "required" else OPTIONAL_MODE_MAX_ATTEMPTS


func _ready():
	var is_debug_mode = OS.has_feature("editor") \
		or OS.get_environment("VIFU_CLI_BRIDGE") != ""
	if not is_debug_mode:
		queue_free()
		return

	_ws = WebSocketPeer.new()
	_active = true
	_connect_to_host()


func _exit_tree():
	_active = false
	if _ws != null:
		_ws.close()


func _connect_to_host():
	var host = OS.get_environment("VIFU_CLI_HOST")
	if host == "":
		host = ProjectSettings.get_setting("vifu/bridge/debug_host", "")
	if host == "":
		return
	if _stopped_retrying:
		return

	var port_env = OS.get_environment("VIFU_CLI_PORT")
	var port = int(port_env) if port_env != "" else int(ProjectSettings.get_setting("vifu/bridge/debug_ws_port", 9876))
	var url = "ws://%s:%d" % [host, port]
	_attempt_count += 1
	print("[WebSocketTransport] Connecting to %s (attempt %d/%d, mode=%s)" % [url, _attempt_count, _max_attempts, _bridge_mode])
	var err = _ws.connect_to_url(url)
	if err != OK:
		push_error("[WebSocketTransport] Failed to initiate connection: " + str(err))
		_stop_retrying_if_exhausted()


func send_raw(json_string: String) -> void:
	if _ws == null or _ws.get_ready_state() != WebSocketPeer.STATE_OPEN:
		return
	var err = _ws.send_text(json_string)
	if err != OK:
		push_error("[WebSocketTransport] Failed to send: " + str(err))


func is_connected_to_host() -> bool:
	return _connected


func _process(delta):
	if _ws == null:
		return

	_ws.poll()

	var state = _ws.get_ready_state()

	match state:
		WebSocketPeer.STATE_OPEN:
			if not _connected:
				_connected = true
				print("[WebSocketTransport] Connected!")
				var global_state = get_node_or_null("/root/GlobalState")
				if global_state and global_state.has_method("send_bridge_ready"):
					global_state.send_bridge_ready("websocket")
				# Re-send loading_complete if preload already finished
				if global_state and global_state.get("preload_complete"):
					global_state.send_loading_complete()

			while _ws.get_available_packet_count() > 0:
				var packet = _ws.get_packet()
				var msg = packet.get_string_from_utf8()
				if msg != "":
					data_received.emit(msg)

		WebSocketPeer.STATE_CLOSED:
			if _connected:
				_connected = false
				print("[WebSocketTransport] Disconnected")

			if _stopped_retrying:
				return

			_reconnect_timer += delta
			if _reconnect_timer >= RECONNECT_INTERVAL:
				_reconnect_timer = 0.0
				_stop_retrying_if_exhausted()
				if _stopped_retrying:
					return
				_connect_to_host()

		WebSocketPeer.STATE_CONNECTING:
			pass

		WebSocketPeer.STATE_CLOSING:
			pass


func _stop_retrying_if_exhausted() -> void:
	if _attempt_count < _max_attempts:
		return
	_stopped_retrying = true
	var message = "[WebSocketTransport] Bridge unavailable after %d attempts" % _attempt_count
	if _bridge_mode == "required":
		push_error(message)
		get_tree().quit(2)
	else:
		print(message + "; continuing without bridge")
