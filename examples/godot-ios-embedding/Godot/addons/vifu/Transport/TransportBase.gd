class_name VifuTransportBase
extends RefCounted

## Abstract base for all bridge transports.
## Subclasses implement send_raw() and emit data_received when data arrives.
## VifuRuntimeBridge picks the right transport at runtime.

signal data_received(json_string: String)

## Send a raw JSON string to the host (Swift, CLI, or web parent).
func send_raw(_json_string: String) -> void:
	push_error("TransportBase.send_raw() not implemented")

## Called by VifuRuntimeBridge when transport should start.
func start() -> void:
	pass

## Called by VifuRuntimeBridge when transport should stop.
func stop() -> void:
	pass

## True when the transport has an active connection.
func is_connected_to_host() -> bool:
	return false
