extends Node3D

var character: Node3D
var elapsed := 0.0
var activity := "loading"
var animation_players: Array[AnimationPlayer] = []
var idle_animation := ""
var talking_animation := ""
var thinking_animation := ""
var status_label: Label
var runtime_hello_request_id := -1


func _ready() -> void:
	_build_environment()
	character = _load_character()
	add_child(character)
	_fit_character(character)
	_collect_animation_players(character)
	_discover_animations()
	_build_status()
	set_activity("idle")
	GlobalState.notify_host("stage_ready", {"character": "default"})


func _process(delta: float) -> void:
	elapsed += delta
	if activity == "idle":
		character.rotation.y = sin(elapsed * 0.45) * 0.025


func handle_host_message(method: String, params: Dictionary) -> void:
	match method:
		"host.activity":
			set_activity(str(params.get("activity", "idle")))
		"runtime_activity":
			set_activity(str(params.get("activity", "idle")))
		"host.runtime.available":
			runtime_hello_request_id = GlobalState.bridge.send_request(
				"runtime.hello",
				{"protocol": "vifu.runtime-bridge/1"}
			)
		"bridge.response":
			_handle_bridge_response(params)
		_:
			if not method.begins_with("runtime.invocation."):
				push_warning("Unknown embedding host method: %s" % method)


func _handle_bridge_response(frame: Dictionary) -> void:
	if str(frame.get("id", "")) != str(runtime_hello_request_id):
		return
	if not bool(frame.get("ok", false)):
		return
	var payload = frame.get("payload", {})
	if typeof(payload) != TYPE_DICTIONARY:
		return
	print("[VifuRuntimeBridge] Connected to project %s" % payload.get("projectId", ""))
	GlobalState.notify_host("stage.runtime.connected", {
		"projectId": payload.get("projectId", ""),
		"protocol": payload.get("protocol", ""),
	})


func set_activity(next_activity: String) -> void:
	activity = next_activity
	if status_label:
		status_label.text = next_activity.capitalize()
	GlobalState.notify_host("stage.activity.changed", {"activity": next_activity})

	if character.has_method("start_talking") and character.has_method("stop_talking"):
		if next_activity == "speaking":
			character.start_talking()
		else:
			character.stop_talking()
		return

	var animation := idle_animation
	match next_activity:
		"thinking":
			animation = thinking_animation if thinking_animation != "" else idle_animation
		"speaking":
			animation = talking_animation if talking_animation != "" else idle_animation
		"loading":
			animation = idle_animation
	_play_animation(animation)


func _load_character() -> Node3D:
	for path in ["res://character.vrm", "res://character.glb"]:
		if ResourceLoader.exists(path):
			var scene := load(path) as PackedScene
			if scene:
				return scene.instantiate()
	return _placeholder_character()


func _placeholder_character() -> Node3D:
	var root := Node3D.new()
	var body := MeshInstance3D.new()
	var body_mesh := CapsuleMesh.new()
	body_mesh.radius = 0.62
	body_mesh.height = 1.9
	body.mesh = body_mesh
	body.position.y = 0.15
	var body_material := StandardMaterial3D.new()
	body_material.albedo_color = Color(0.16, 0.18, 0.23)
	body.material_override = body_material
	root.add_child(body)

	var head := MeshInstance3D.new()
	var head_mesh := SphereMesh.new()
	head_mesh.radius = 0.72
	head_mesh.height = 1.44
	head.mesh = head_mesh
	head.position.y = 1.35
	var head_material := StandardMaterial3D.new()
	head_material.albedo_color = Color(0.96, 0.73, 0.62)
	head.material_override = head_material
	root.add_child(head)
	return root


func _fit_character(root: Node3D) -> void:
	var meshes: Array[MeshInstance3D] = []
	_collect_meshes(root, meshes)
	if meshes.is_empty():
		return

	var inverse := root.global_transform.affine_inverse()
	var bounds: AABB
	var has_bounds := false
	for mesh in meshes:
		var local_bounds: AABB = (inverse * mesh.global_transform) * mesh.get_aabb()
		if not has_bounds:
			bounds = local_bounds
			has_bounds = true
		else:
			bounds = bounds.merge(local_bounds)

	if bounds.size.y <= 0.001:
		return
	var target_height := 3.35
	var scale_factor := target_height / bounds.size.y
	root.scale = Vector3.ONE * scale_factor
	var center := bounds.get_center()
	root.position = Vector3(
		-center.x * scale_factor,
		-1.08 - bounds.position.y * scale_factor,
		-center.z * scale_factor
	)


func _collect_meshes(node: Node, result: Array[MeshInstance3D]) -> void:
	if node is MeshInstance3D:
		result.append(node)
	for child in node.get_children():
		_collect_meshes(child, result)


func _collect_animation_players(node: Node) -> void:
	if node is AnimationPlayer:
		animation_players.append(node)
	for child in node.get_children():
		_collect_animation_players(child)


func _discover_animations() -> void:
	for player in animation_players:
		for animation_name in player.get_animation_list():
			var normalized := str(animation_name).to_lower()
			if normalized == "reset":
				continue
			if idle_animation == "":
				idle_animation = str(animation_name)
			if "idle" in normalized or "default" in normalized:
				idle_animation = str(animation_name)
			if talking_animation == "" and (
				"talk" in normalized or "speak" in normalized or "happy" in normalized
			):
				talking_animation = str(animation_name)
			if thinking_animation == "" and (
				"think" in normalized or "confuse" in normalized
			):
				thinking_animation = str(animation_name)


func _play_animation(animation_name: String) -> void:
	if animation_name == "":
		return
	for player in animation_players:
		if player.has_animation(animation_name):
			player.play(animation_name)


func _build_status() -> void:
	var canvas := CanvasLayer.new()
	add_child(canvas)
	var character_label := Label.new()
	character_label.name = "CharacterName"
	character_label.text = "Vifu Godot iOS Starter"
	character_label.position = Vector2(20, 20)
	character_label.add_theme_font_size_override("font_size", 22)
	canvas.add_child(character_label)
	var panel := PanelContainer.new()
	panel.position = Vector2(20, 54)
	panel.custom_minimum_size = Vector2(140, 42)
	canvas.add_child(panel)
	status_label = Label.new()
	status_label.name = "Activity"
	status_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	status_label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	status_label.add_theme_font_size_override("font_size", 16)
	panel.add_child(status_label)


func _build_environment() -> void:
	var camera := Camera3D.new()
	camera.position = Vector3(0, 0.62, 5.65)
	camera.fov = 38
	camera.current = true
	add_child(camera)

	var world := WorldEnvironment.new()
	var environment := Environment.new()
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = Color(0.055, 0.06, 0.075)
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Color(0.45, 0.52, 0.70)
	environment.ambient_light_energy = 0.8
	world.environment = environment
	add_child(world)

	var key := OmniLight3D.new()
	key.position = Vector3(-2, 3, 3)
	key.light_color = Color(1.0, 0.48, 0.36)
	key.light_energy = 6.0
	add_child(key)

	var floor := MeshInstance3D.new()
	var floor_mesh := PlaneMesh.new()
	floor_mesh.size = Vector2(12, 12)
	floor.mesh = floor_mesh
	floor.position.y = -1.08
	var floor_material := StandardMaterial3D.new()
	floor_material.albedo_color = Color(0.07, 0.075, 0.09)
	floor_material.roughness = 0.9
	floor.material_override = floor_material
	add_child(floor)
