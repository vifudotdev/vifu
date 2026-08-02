from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from vifu_feedback import VifuFeedback, invocation_id_from_info, invocation_id_from_response


INVOCATION_ID = "1ec1578e-7527-45eb-86d3-d844dedfd75a"


class _Response:
    def __init__(self, *, response_id: str | None = None, headers=None, status: int = 202):
        self.id = response_id
        self.headers = headers or {}
        self.status = status

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return None


class VifuFeedbackTest(unittest.TestCase):
    def setUp(self) -> None:
        self.feedback = VifuFeedback(
            "http://127.0.0.1:6790/", "stardojo", "project-key"
        )

    def test_reads_invocation_id_from_header_or_completion_id(self) -> None:
        self.assertEqual(
            invocation_id_from_response(
                _Response(headers={"X-Vifu-Invocation-Id": INVOCATION_ID})
            ),
            INVOCATION_ID,
        )
        self.assertEqual(
            invocation_id_from_response(_Response(response_id=f"chatcmpl-{INVOCATION_ID}")),
            INVOCATION_ID,
        )
        self.assertEqual(
            invocation_id_from_info({"vifu_invocation_id": INVOCATION_ID}), INVOCATION_ID
        )

    @patch("vifu_feedback._open_without_redirects")
    def test_reports_parser_path_with_project_auth(self, open_feedback) -> None:
        open_feedback.return_value = _Response()
        result = self.feedback.output_accepted(
            INVOCATION_ID, {"reasoning": "ok"}, required_keys=("actions",)
        )

        outgoing = open_feedback.call_args.args[0]
        self.assertEqual(result.outcome, "fail")
        self.assertEqual(
            outgoing.full_url,
            f"http://127.0.0.1:6790/stardojo/v1/traces/{INVOCATION_ID}/feedback",
        )
        self.assertEqual(outgoing.get_header("Authorization"), "Bearer project-key")
        self.assertEqual(json.loads(outgoing.data), {
            "event": "OUTPUT_ACCEPTED",
            "outcome": "fail",
            "message": "StarDojo response is missing required key actions",
            "path": "$.actions",
        })

    @patch("vifu_feedback._open_without_redirects")
    def test_maps_action_and_frame_results(self, open_feedback) -> None:
        open_feedback.return_value = _Response()
        action = self.feedback.action_applied(
            INVOCATION_ID,
            {"errors": True, "errors_info": "invalid skill", "executed_skills": []},
        )
        self.assertEqual(action.outcome, "fail")

        with tempfile.TemporaryDirectory() as directory:
            screenshot = Path(directory, "frame.png")
            screenshot.write_bytes(b"png")
            frame = self.feedback.frame_presented(
                INVOCATION_ID, screenshot, presented=True
            )
        self.assertEqual(frame.outcome, "pass")

    def test_rejects_plaintext_remote_feedback_and_ambiguous_success(self) -> None:
        with self.assertRaisesRegex(ValueError, "loopback"):
            VifuFeedback("http://example.com", "stardojo", "project-key")

        with patch("vifu_feedback._open_without_redirects") as open_feedback:
            open_feedback.return_value = _Response()
            action = self.feedback.action_applied(INVOCATION_ID, {"errors": False})
            string_action = self.feedback.action_applied(
                INVOCATION_ID,
                {"errors": False, "executed_skills": "move_left"},
            )
            frame = self.feedback.frame_presented(INVOCATION_ID, None)
        self.assertEqual(action.outcome, "unknown")
        self.assertEqual(string_action.outcome, "unknown")
        self.assertEqual(frame.outcome, "unknown")


if __name__ == "__main__":
    unittest.main()
