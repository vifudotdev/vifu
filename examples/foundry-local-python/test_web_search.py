from __future__ import annotations

import unittest
from unittest import mock

from main import find_sources, normalize_search_query, search_web


RSS = b"""<?xml version="1.0"?>
<rss><channel><item>
  <title>Arm on-device AI</title>
  <link>https://example.test/arm-ai</link>
  <description>Fast &amp; &lt;b&gt;local&lt;/b&gt; inference.</description>
</item></channel></rss>
"""


class WebSearchTests(unittest.TestCase):
    def test_returns_citable_sources_from_the_search_feed(self) -> None:
        response = mock.MagicMock()
        response.__enter__.return_value.read.return_value = RSS
        with mock.patch("urllib.request.urlopen", return_value=response) as open_url:
            results = search_web("Arm AI", limit=3)

        self.assertEqual(
            results,
            [{
                "title": "Arm on-device AI",
                "url": "https://example.test/arm-ai",
                "snippet": "Fast & local inference.",
            }],
        )
        request = open_url.call_args.args[0]
        self.assertIn("q=Arm+AI", request.full_url)
        self.assertIn("format=rss", request.full_url)

    def test_rejects_empty_queries(self) -> None:
        with self.assertRaises(ValueError):
            search_web("  ")

    def test_current_research_uses_an_english_news_feed(self) -> None:
        response = mock.MagicMock()
        response.__enter__.return_value.read.return_value = RSS
        with mock.patch("urllib.request.urlopen", return_value=response) as open_url:
            search_web("Arm on-device AI", current=True)

        request = open_url.call_args.args[0]
        self.assertIn("news.google.com/rss/search", request.full_url)
        self.assertIn("hl=en-US", request.full_url)
        self.assertIn("ceid=US%3Aen", request.full_url)

    def test_normalizes_a_quoted_conversational_query(self) -> None:
        self.assertEqual(
            normalize_search_query('just, "Recently, in Tokyo, what happened?"'),
            "Recently, in Tokyo, what happened?",
        )

    def test_falls_back_to_web_search_when_news_has_no_results(self) -> None:
        source = {
            "title": "Tokyo update",
            "url": "https://example.test/tokyo",
            "snippet": "A current update.",
        }
        with mock.patch("main.search_web", side_effect=[[], [source]]) as search:
            search_query, mode, sources = find_sources(
                'just, "Recently, in Tokyo, what happened?"'
            )

        self.assertEqual(search_query, "Recently, in Tokyo, what happened?")
        self.assertEqual(mode, "web")
        self.assertEqual(sources, [source])
        self.assertEqual(
            [call.kwargs["current"] for call in search.call_args_list],
            [True, False],
        )


if __name__ == "__main__":
    unittest.main()
