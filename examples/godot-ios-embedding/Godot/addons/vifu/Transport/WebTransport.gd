extends Node

## Web runtime transport for HTML5 exports.
## Uses postMessage between the embedded Godot iframe and the parent page.
## Replaces the previous WebBridgeClient.gd in Core/.
##
## Must be added to the scene tree (extends Node for _process polling).
## Self-disables when not running in a web browser.

signal data_received(json_string: String)

var _installed: bool = false


func _ready():
	if not OS.has_feature("web"):
		queue_free()
		return

	_install_bridge()


func _process(_delta):
	if not _installed:
		return

	var next_message = JavaScriptBridge.eval(
		"(function(){ if (!window.__vifuBridgeInbound || window.__vifuBridgeInbound.length === 0) return ''; return window.__vifuBridgeInbound.shift(); })()",
		true
	)

	if next_message == null:
		return

	var raw = str(next_message)
	if raw == "":
		return

	data_received.emit(raw)


func send_raw(json_string: String) -> void:
	if not _installed:
		return

	var encoded_message = JSON.stringify(json_string)
	JavaScriptBridge.eval(
		"(function(){ if (window.__vifuRuntimeBridgeSend) { window.__vifuRuntimeBridgeSend(%s); } })()" % encoded_message,
		false
	)


func is_connected_to_host() -> bool:
	return _installed


func _install_bridge():
	if _installed:
		return

	JavaScriptBridge.eval("""
(() => {
	if (window.__vifuRuntimeBridgeInstalled) {
		return;
	}
	window.__vifuBridgeInbound = window.__vifuBridgeInbound || [];
	window.addEventListener("message", (event) => {
		const data = event.data || {};
		if (data.source !== "vifu-web-host") {
			return;
		}
		if (typeof data.message === "string") {
			window.__vifuBridgeInbound.push(data.message);
			return;
		}
		if (data.message && typeof data.message === "object") {
			window.__vifuBridgeInbound.push(JSON.stringify(data.message));
		}
	});
	window.__vifuRuntimeBridgeSend = (message) => {
		if (!window.parent) {
			return;
		}
		window.parent.postMessage(
			{ source: "vifu-engine-runtime", message },
			"*"
		);
	};
	window.__vifuRuntimeBridgeInstalled = true;
	window.__vifuRuntimeBridgeSend(JSON.stringify({
		type: "event",
		event: "bridge.ready",
		payload: { transport: "iframe-postmessage" }
	}));
})();
""", false)

	_installed = true
