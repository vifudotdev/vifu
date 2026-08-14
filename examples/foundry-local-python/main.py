from __future__ import annotations

from pathlib import Path

from foundry_local_sdk import Configuration, FoundryLocalManager

from vifu import Vifu
from vifu.integrations.foundry import trace_foundry_stream
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
        return {"query": request.input["query"], "sources": search_web(request.input["query"])}

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
                "Write a concise research brief from the web sources below. "
                "Cite factual claims with source numbers such as [1]. Do not invent facts.\n\n"
                f"Question: {request.input['question']}\n\n{source_text}"
            ),
        }]
        chunks = client.complete_streaming_chat(messages)
        observed = trace_foundry_stream(request, chunks, model=MODEL)
        answer = "".join(
            chunk.choices[0].delta.content or ""
            for chunk in observed
        )
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
        print(report.output["brief"])
        print("\nSources:")
        for source in report.output["sources"]:
            print(f"- {source['title']}: {source['url']}")

    try:
        app.run(run_my_app)
    finally:
        model.unload()


if __name__ == "__main__":
    main()
