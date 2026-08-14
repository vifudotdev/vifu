"""Small dependency-free web search used by the Web Research example."""

from __future__ import annotations

import html
import json
import re
import sys
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET


def search_web(
    query: str,
    limit: int = 5,
    *,
    current: bool = False,
) -> list[dict[str, str]]:
    query = query.strip()
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


def clean_text(value: str) -> str:
    without_tags = re.sub(r"<[^>]+>", " ", html.unescape(value))
    return re.sub(r"\s+", " ", without_tags).strip()


if __name__ == "__main__":
    query = " ".join(sys.argv[1:]).strip() or "Arm-optimized on-device AI"
    print(json.dumps(search_web(query, current=True), indent=2, ensure_ascii=False))
