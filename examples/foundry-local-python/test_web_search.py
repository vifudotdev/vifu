from __future__ import annotations

import unittest
from unittest import mock

from web_search import search_web


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


if __name__ == "__main__":
    unittest.main()
