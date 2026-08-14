from __future__ import annotations

import html
import re
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

MODEL = "qwen2.5-0.5b"
SEARCH_QUOTES = str.maketrans("", "", "\"'“”‘’")


def normalize_search_query(query: str) -> str:
    query = query.strip()
    query = re.sub(r"^(?:just|please)\s*[,，:：]\s*", "", query, flags=re.IGNORECASE)
    query = query.translate(SEARCH_QUOTES)
    return re.sub(r"\s+", " ", query).strip()


def search_web(
    query: str,
    limit: int = 5,
    *,
    current: bool = False,
) -> list[dict[str, str]]:
    query = normalize_search_query(query)
    if not query:
        raise ValueError("query must not be empty")
    if limit < 1 or limit > 10:
        raise ValueError("limit must be between 1 and 10")
    if current:
        url = "https://news.google.com/rss/search?" + urllib.parse.urlencode({
            "q": query,
            "hl": "en-US",
            "gl": "US",
            "ceid": "US:en",
        })
    else:
        url = "https://www.bing.com/search?" + urllib.parse.urlencode({
            "q": query,
            "format": "rss",
            "setlang": "en-US",
            "cc": "US",
        })
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "Vifu Web Research Example/0.1"},
    )
    with urllib.request.urlopen(request, timeout=15) as response:
        root = ET.fromstring(response.read())
    results = []
    for item in root.findall("./channel/item")[:limit]:
        results.append(
            {
                "title": item.findtext("title", default="").strip(),
                "url": item.findtext("link", default="").strip(),
                "snippet": clean_text(item.findtext("description", default="")),
            }
        )
    return [result for result in results if result["title"] and result["url"]]


def find_sources(query: str, limit: int = 5) -> tuple[str, str, list[dict[str, str]]]:
    search_query = normalize_search_query(query)
    for mode, current in (("news", True), ("web", False)):
        sources = search_web(search_query, limit=limit, current=current)
        if sources:
            return search_query, mode, sources
    return search_query, "none", []


def clean_text(value: str) -> str:
    without_tags = re.sub(r"<[^>]+>", " ", html.unescape(value))
    return re.sub(r"\s+", " ", without_tags).strip()


def main() -> None:
    from foundry_local_sdk import Configuration, FoundryLocalManager

    from vifu import Vifu
    from vifu.integrations.foundry import foundry_chunk_text, trace_foundry_stream

    # Normal Foundry Local setup remains unchanged.
    FoundryLocalManager.initialize(Configuration(app_name="web-research"))
    model = FoundryLocalManager.instance.catalog.get_model(MODEL)
    model.download()
    model.load()
    client = model.get_chat_client()

    app = Vifu(
        "Web Research",
        workspace=Path.cwd(),
        capture_trace_content=True,
    )

    @app.agent(
        "web-search",
        name="Web Search",
        capability="search",
        metadata={
            "providerName": "Python Web Search",
            "providerSettings": {"sources": ["Google News RSS", "Bing RSS"]},
        },
    )
    def web_search(request):
        search_query, search_mode, sources = find_sources(request.input["query"])
        return {
            "query": request.input["query"],
            "searchQuery": search_query,
            "searchMode": search_mode,
            "sources": sources,
        }

    @app.agent(
        "researcher",
        name="Local Researcher",
        capability="research",
        metadata={
            "framework": "foundry-local",
            "model": MODEL,
            "providerName": "Foundry Local",
            "providerSettings": {"framework": "Foundry Local", "model": MODEL},
        },
        instructions=(
            "Write a concise research brief using only the supplied sources. "
            "Use one bullet per supported finding and begin every bullet with "
            "its source number, such as [1]. Do not add unsupported claims."
        ),
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
                f"{request.instructions or ''}\n\n"
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
        print("\nEnter a topic to research. Type 'exit' or press Ctrl+C to stop.")
        research_number = 0
        while True:
            try:
                question = input("\nResearch> ").strip()
            except EOFError:
                return
            if question.lower() in {"exit", "quit"}:
                return
            if not question:
                continue

            research_number += 1
            session_id = f"research-{research_number}"
            try:
                print("Searching the web...")
                search = vifu.invoke(
                    "web-search",
                    {"query": question},
                    session_id=session_id,
                )
                search_query = search.output["searchQuery"]
                if search_query != question:
                    print(f"Search query: {search_query}")
                sources = search.output["sources"]
                if not sources:
                    print(
                        "No citable sources were returned by either news or web "
                        "search. Try a more specific topic."
                    )
                    continue

                print("Researching with the local model...")
                report = vifu.invoke(
                    "researcher",
                    {"question": question, "sources": sources},
                    session_id=session_id,
                )
            except Exception as error:
                print(f"Research failed: {error}")
                continue

            print("\nResearch brief:\n")
            print(report.output["brief"])
            print("\nSources:")
            for index, source in enumerate(report.output["sources"], start=1):
                print(f"[{index}] {source['title']}: {source['url']}")

    try:
        app.run(run_my_app)
    finally:
        model.unload()


if __name__ == "__main__":
    main()
