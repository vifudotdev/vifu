extends Node

signal godot_message_to_swift(message: String)

var preload_complete := false
var bridge: Node


func _ready() -> void:
	var bridge_script = load("res://addons/vifu/Core/RuntimeBridge.gd")
	bridge = bridge_script.new()
	bridge.name = "VifuRuntimeBridge"
	add_child(bridge)
	bridge.message_received.connect(_on_bridge_message)
	call_deferred("_finish_loading")


func _finish_loading() -> void:
	preload_complete = true
	send_loading_complete()
	send_bridge_ready("standalone")


func handle_swift_message(json_string: String) -> void:
	if bridge == null or not bridge.has_method("push_inbound_message"):
		push_error("Vifu Runtime Bridge is not ready")
		return
	bridge.push_inbound_message(json_string)


func _on_bridge_message(method: String, params: Dictionary) -> void:
	match method:
		"take_screenshot":
			_handle_take_screenshot(params)
		"take_snapshot":
			notify_host("snapshot_result", {"labels": _collect_visible_labels()}, true)
		_:
			var scene := get_tree().current_scene
			if scene != null and scene.has_method("handle_host_message"):
				scene.handle_host_message(method, params)


func notify_host(event_name: String, payload: Dictionary = {}, legacy_jsonrpc := false) -> void:
	if legacy_jsonrpc:
		godot_message_to_swift.emit(JSON.stringify({
			"jsonrpc": "2.0",
			"method": event_name,
			"params": payload,
		}))
		return
	godot_message_to_swift.emit(JSON.stringify({
		"type": "event",
		"event": event_name,
		"payload": payload,
	}))


func send_loading_complete() -> void:
	notify_host("loading_complete", {}, true)


func send_bridge_ready(transport: String) -> void:
	notify_host("bridge_ready", {
		"transport": transport,
		"preload_complete": preload_complete,
	}, true)


func _handle_take_screenshot(payload: Dictionary) -> void:
	var save_path = payload.get("path", "/tmp/vifu-godot-ios-starter.png")
	await RenderingServer.frame_post_draw
	var image := get_viewport().get_texture().get_image()
	if image == null:
		notify_host("screenshot_error", {"error": "Failed to get viewport image"}, true)
		return
	var error := image.save_png(save_path)
	if error == OK:
		notify_host("screenshot_taken", {"path": save_path}, true)
	else:
		notify_host("screenshot_error", {"error": str(error)}, true)


func _collect_visible_labels() -> Array:
	var result := []
	_walk_labels(get_tree().root, result)
	return result


func _walk_labels(node: Node, result: Array) -> void:
	if node is Label and node.visible:
		result.append({
			"name": node.name,
			"text": node.text,
			"path": str(node.get_path()),
		})
	if node is RichTextLabel and node.visible:
		result.append({
			"name": node.name,
			"text": node.text,
			"path": str(node.get_path()),
		})
	for child in node.get_children():
		_walk_labels(child, result)
