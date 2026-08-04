class_name VifuInProcessTransport
extends "res://addons/vifu/Transport/TransportBase.gd"

## In-process transport for SwiftGodotKit (iOS release builds).
## Wraps the GlobalState.godot_message_to_swift signal for outbound messages.
## Inbound messages arrive via GlobalState.handle_swift_message() which
## is called directly by SwiftGodot signals.
##
## This transport is a thin adapter: it connects outbound sends to the
## existing signal, and receives inbound data pushed from the bridge.

var _active: bool = false


func start() -> void:
	_active = true


func stop() -> void:
	_active = false


func send_raw(json_string: String) -> void:
	if not _active:
		return
	# Emit via GlobalState signal so SwiftGodotKit picks it up
	var global_state = _get_global_state()
	if global_state:
		global_state.emit_signal("godot_message_to_swift", json_string)


func is_connected_to_host() -> bool:
	return _active


## Called by VifuRuntimeBridge when an inbound message arrives via
## GlobalState.handle_swift_message(). This pushes it into the
## transport's data_received signal.
func push_inbound(json_string: String) -> void:
	data_received.emit(json_string)


func _get_global_state() -> Node:
	var loop = Engine.get_main_loop()
	if loop is SceneTree:
		return loop.root.get_node_or_null("GlobalState")
	return null
