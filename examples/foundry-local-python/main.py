from __future__ import annotations

import time
from pathlib import Path

from foundry_local_sdk import Configuration, FoundryLocalManager

from vifu import Vifu
from vifu.integrations.foundry import foundry_chunk_text, trace_foundry_stream
from web_search import search_web


MODEL = "qwen2.5-0.5b"


def main() -> None:
    # Normal Foundry Local setup remains unchanged.
    FoundryLocalManager.initialize(Configuration(app_name="web-research"))
    model = FoundryLocalManager.instance.catalog.get_model(MODEL)
    model.download()
    model.load()
    client = model.get_chat_client()

    app = Vifu(
        "Web Research",
        workspace=Path(__file__).parent,
        capture_trace_content=True,
    )

    @app.agent("web-search", name="Web Search", capability="search")
    def web_search(request):
        return {
            "query": request.input["query"],
            "sources": search_web(request.input["query"], current=True),
        }

    @app.agent(
        "researcher",
        name="Local Researcher",
        capability="research",
        metadata={"framework": "foundry-local", "model": MODEL},
    )
    def researcher(request):
        sources = request.input["sources"]
        source_text = "\n\n".join(
            f"[{index}] {source['title']}\nURL: {source['url']}\n{source['snippet']}"
            for index, source in enumerate(sources, start=1)
        )
        messages = [{
            "role": "user",
            "content": (
                "Write a concise research draft using only the sources below. "
                "Use one bullet per supported finding and begin every bullet with its "
                "source number, such as [1]. If a source does not support a finding, "
                "do not include it.\n\n"
                f"Question: {request.input['question']}\n\n{source_text}"
            ),
        }]
        chunks = client.complete_streaming_chat(messages)
        observed = trace_foundry_stream(request, chunks, model=MODEL)
        answer = "".join(foundry_chunk_text(chunk) for chunk in observed)
        if not answer.strip():
            raise RuntimeError(f"Foundry Local model {MODEL} returned no text")
        return {"brief": answer, "sources": sources}

    def run_my_app(vifu: Vifu) -> None:
        question = "What are the latest practical uses of Arm-optimized on-device AI?"
        search = vifu.invoke(
            "web-search",
            {"query": question},
            session_id="arm-research",
        )
        report = vifu.invoke(
            "researcher",
            {"question": question, "sources": search.output["sources"]},
            session_id="arm-research",
        )
        print("Local research draft:\n")
        print(report.output["brief"])
        print("\nSources:")
        for index, source in enumerate(report.output["sources"], start=1):
            print(f"[{index}] {source['title']}: {source['url']}")
        print("\nWeb Research is running. Press Ctrl+C to stop.")
        while True:
            time.sleep(3_600)

    try:
        app.run(run_my_app)
    finally:
        model.unload()


if __name__ == "__main__":
    main()
