from foundry_local_sdk import Configuration, FoundryLocalManager

from vifu import VifuRuntime

from provider import register_foundry_agent


MODEL = "qwen2.5-0.5b"

FoundryLocalManager.initialize(Configuration(app_name="vifu-foundry-local"))
manager = FoundryLocalManager.instance
manager.download_and_register_eps(progress_callback=lambda _name, _percent: None)
model = manager.catalog.get_model(MODEL)
model.download(lambda _progress: None)
model.load()

runtime = VifuRuntime("foundry-local-python")
register_foundry_agent(runtime, model.get_chat_client(), model=MODEL)

result = runtime.invoke("foundry-chat", {"prompt": "Explain local inference in one sentence."})
print(result.output["text"])
print({"invocation_id": result.invocation_id, "stages": result.trace})

model.unload()
