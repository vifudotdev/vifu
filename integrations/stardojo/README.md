# StarDojo trace feedback

This adapter connects StarDojo's application boundaries to the same Vifu trace
that served its OpenAI-compatible completion. It reports three facts:

- whether StarDojo parsed the model output and found the required keys;
- whether the requested game action ran;
- whether the following screenshot was produced.

`vifu_feedback.py` uses only Python's standard library. Keep it on StarDojo's
Python path, then initialize it with the same Vifu server, project, and project
key used by the OpenAI client:

```python
feedback = VifuFeedback(
    server_url="http://127.0.0.1:6790",
    project_slug="stardojo",
    project_key=os.environ["VIFU_PROJECT_KEY"],
)
telemetry = VifuFeedbackWorker(feedback)
```

At the completion call site, retain Vifu's response ID in StarDojo's existing
`info` map:

```python
info["vifu_invocation_id"] = invocation_id_from_response(response)
```

Report the existing parser, executor, and next-frame results:

```python
invocation_id = invocation_id_from_info(info)

try:
    parsed = parse_semi_formatted_text(message)
except Exception as exc:
    telemetry.parser_failed(invocation_id, exc)
    raise
else:
    telemetry.output_accepted(invocation_id, parsed, required_keys=("actions",))

execution = game_manager.execute_actions(parsed["actions"], skill_executor)
telemetry.action_applied(invocation_id, execution)

screenshot_path = game_manager.capture_screen()
frame_ok = verify_next_frame(screenshot_path)  # StarDojo/game-specific assertion
telemetry.frame_presented(invocation_id, screenshot_path, presented=frame_ok)
```

The bounded background worker keeps feedback delivery off StarDojo's parser,
action, and render path. Supply `on_error` when delivery failures should be
logged, and call `telemetry.close()` during runner shutdown. The
Vifu TUI then distinguishes model execution, response validation, application
parsing, action execution, and frame capture instead of presenting one flat log.

Run the adapter checks from this directory:

```console
python -m unittest -v test_vifu_feedback.py
```
