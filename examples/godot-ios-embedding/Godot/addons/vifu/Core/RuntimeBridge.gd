class_name VifuRuntimeBridge
extends Node

## Singleton bridge — single I/O point for all host communication.
## Auto-detects environment and selects the right transport:
##   Web browser       -> WebTransport (postMessage)
##   Editor + debug    -> WebSocketTransport (ws://host:9876)
##   SwiftGodotKit     -> InProcessTransport (signal-based)
##
## Outbound: send(event, payload) or send_request(method, params) -> transport
## Inbound:  transport -> message_received signal -> the game host adapter

signal message_received(method: String, params: Dictionary)

var _transport_node: Node = null
var _in_process_transport = null
var _request_id_counter: int = 0


func _resolve_bridge_mode() -> String:
	var explicit_mode = OS.get_environment("VIFU_BRIDGE_MODE").strip_edges().to_lower()
	if explicit_mode in ["disabled", "optional", "required"]:
		return explicit_mode
	if OS.get_environment("VIFU_CLI_BRIDGE") != "":
		return "required"
	if OS.has_feature("editor"):
		return "optional"
	return "disabled"


func _ready():
	_setup_transport()
	# Connect to GlobalState outbound signal for backwards compatibility.
	# Any code still calling GlobalState.send_to_swift() gets forwarded
	# through the active transport automatically.
	var global_state = _get_global_state()
	if global_state and global_state.has_signal("godot_message_to_swift"):
		if not global_state.godot_message_to_swift.is_connected(_on_global_state_outbound):
			global_state.godot_message_to_swift.connect(_on_global_state_outbound)


func _setup_transport():
	var bridge_mode = _resolve_bridge_mode()
	if OS.has_feature("web"):
		# Web: postMessage transport
		var web = load("res://addons/vifu/Transport/WebTransport.gd")
		if web:
			_transport_node = web.new()
			_transport_node.name = "WebTransport"
			add_child(_transport_node)
			_transport_node.data_received.connect(_on_data_received)
			print("[VifuRuntimeBridge] Using WebTransport")
			return

	if bridge_mode != "disabled":
		var host = ProjectSettings.get_setting("vifu/bridge/debug_host", "")
		if host != "":
			# Editor debug: WebSocket transport
			var ws = load("res://addons/vifu/Transport/WebSocketTransport.gd")
			if ws:
				_transport_node = ws.new()
				if _transport_node.has_method("configure"):
					_transport_node.configure(bridge_mode)
				_transport_node.name = "WebSocketTransport"
				add_child(_transport_node)
				_transport_node.data_received.connect(_on_data_received)
				print("[VifuRuntimeBridge] Using WebSocketTransport (mode: %s, host: %s)" % [bridge_mode, host])
				return

	# Default: in-process (SwiftGodotKit)
	var in_process_script = load("res://addons/vifu/Transport/InProcessTransport.gd")
	if in_process_script:
		_in_process_transport = in_process_script.new()
		_in_process_transport.start()
		_in_process_transport.data_received.connect(_on_data_received)
	print("[VifuRuntimeBridge] Using InProcessTransport (mode: %s)" % bridge_mode)


## Send a Vifu protocol event (no response expected).
func send(event_name: String, payload: Dictionary = {}) -> void:
	var msg = JSON.stringify({"type": "event", "event": event_name, "payload": payload})
	_send_raw(msg)


## Send a Vifu protocol request (expects a response). Returns the request ID.
func send_request(method: String, params: Dictionary = {}) -> int:
	_request_id_counter += 1
	var msg = JSON.stringify({
		"type": "req",
		"method": method,
		"params": params,
		"id": str(_request_id_counter),
	})
	_send_raw(msg)
	return _request_id_counter


## Send a pre-built JSON string directly.
func send_raw_json(json_string: String) -> void:
	_send_raw(json_string)


## Push an inbound message from the host into the bridge.
## Used by GlobalState.handle_swift_message() in in-process mode.
func push_inbound_message(json_string: String) -> void:
	_on_data_received(json_string)


func _send_raw(json_string: String) -> void:
	if _transport_node and _transport_node.has_method("send_raw"):
		_transport_node.send_raw(json_string)
	elif _in_process_transport:
		_in_process_transport.send_raw(json_string)
	else:
		# Fallback: emit via GlobalState for backwards compatibility
		var global_state = _get_global_state()
		if global_state:
			global_state.emit_signal("godot_message_to_swift", json_string)


func _on_data_received(json_string: String) -> void:
	var json = JSON.new()
	if json.parse(json_string) != OK:
		push_error("[VifuRuntimeBridge] Failed to parse: " + json_string)
		return

	var data = json.data
	if typeof(data) != TYPE_DICTIONARY:
		return

	var method: String = ""
	var params: Dictionary = {}

	if data.get("type") == "req":
		method = data.get("method", "")
		var p = data.get("params", {})
		if typeof(p) == TYPE_DICTIONARY:
			params = p
	elif data.get("type") == "event":
		method = data.get("event", "")
		var p = data.get("payload", {})
		if typeof(p) == TYPE_DICTIONARY:
			params = p
	elif data.get("type") == "res":
		method = "bridge.response"
		params = data
	elif data.get("jsonrpc") == "2.0":
		# Legacy diagnostic clients may still use JSON-RPC.
		method = data.get("method", "")
		var p = data.get("params", {})
		if typeof(p) == TYPE_DICTIONARY:
			params = p
	else:
		# Legacy {v,t,p} format — still accepted during transition
		method = data.get("t", "")
		var p = data.get("p", {})
		if typeof(p) == TYPE_DICTIONARY:
			params = p

	if method == "":
		return

	message_received.emit(method, params)


## Handle outbound messages from GlobalState.godot_message_to_swift signal.
## Forwards them through the active transport (WebSocket/Web/InProcess).
func _on_global_state_outbound(json_string: String) -> void:
	# Only forward through transport node (not InProcessTransport, which
	# already emits back via GlobalState to avoid infinite loop)
	if _transport_node and _transport_node.has_method("send_raw"):
		_transport_node.send_raw(json_string)


func _get_global_state() -> Node:
	return get_node_or_null("/root/GlobalState")
