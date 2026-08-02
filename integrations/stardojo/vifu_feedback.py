"""Small StarDojo-to-Vifu application feedback bridge.

The adapter deliberately reports only the three application boundaries used by
Vifu's trace inspector. It uses Python's standard library so StarDojo does not
need another runtime dependency.
"""

from __future__ import annotations

from dataclasses import dataclass
import ipaddress
import json
from pathlib import Path
from queue import Full, Queue
import re
from threading import Thread
from typing import Any, Callable, Mapping, Sequence
from urllib import error, parse, request
from uuid import UUID


_EVENTS = frozenset({"OUTPUT_ACCEPTED", "ACTION_APPLIED", "FRAME_PRESENTED"})
_OUTCOMES = frozenset({"pass", "fail", "unknown", "notApplicable"})
_MAX_MESSAGE_LENGTH = 512
_MAX_PATH_LENGTH = 256


def invocation_id_from_response(response: Any) -> str:
    """Return Vifu's canonical invocation UUID from an OpenAI response.

    A raw OpenAI SDK response exposes ``X-Vifu-Invocation-Id`` in its headers.
    A parsed response exposes the same identifier as ``chatcmpl-<uuid>``.
    """

    headers = getattr(response, "headers", None)
    if headers is not None:
        value = headers.get("X-Vifu-Invocation-Id")
        if value:
            return _canonical_uuid(value)

    response_id = getattr(response, "id", None)
    if response_id is None and isinstance(response, Mapping):
        response_id = response.get("id")
    if not isinstance(response_id, str):
        raise ValueError("completion response does not contain a Vifu invocation id")
    return _canonical_uuid(response_id.removeprefix("chatcmpl-"))


def invocation_id_from_info(info: Mapping[str, Any]) -> str:
    """Read an invocation UUID saved in StarDojo's completion ``info`` map."""

    value = info.get("vifu_invocation_id") or info.get("response_id")
    if not isinstance(value, str):
        raise ValueError("completion info does not contain vifu_invocation_id")
    return _canonical_uuid(value.removeprefix("chatcmpl-"))


@dataclass(frozen=True, slots=True)
class FeedbackResult:
    event: str
    outcome: str
    status_code: int


class VifuFeedback:
    """Reports StarDojo parser, action, and frame boundaries to one Vifu trace."""

    def __init__(
        self,
        server_url: str,
        project_slug: str,
        project_key: str,
        *,
        timeout_seconds: float = 2.0,
    ) -> None:
        if not server_url.strip():
            raise ValueError("server_url is required")
        if not project_slug.strip():
            raise ValueError("project_slug is required")
        if not project_key.strip():
            raise ValueError("project_key is required")
        if any(ord(character) < 33 or ord(character) > 126 for character in project_key):
            raise ValueError("project_key contains invalid characters")
        if timeout_seconds <= 0:
            raise ValueError("timeout_seconds must be greater than zero")
        self._server_url = _validated_server_url(server_url)
        normalized_slug = project_slug.strip()
        if re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,127}", normalized_slug) is None:
            raise ValueError("project_slug contains invalid characters")
        self._project_slug = parse.quote(normalized_slug, safe="")
        self._project_key = project_key.strip()
        self._timeout_seconds = timeout_seconds

    def report(
        self,
        invocation_id: str,
        event: str,
        outcome: str,
        *,
        message: str | None = None,
        path: str | None = None,
    ) -> FeedbackResult:
        invocation_id = _canonical_uuid(invocation_id)
        if event not in _EVENTS:
            raise ValueError(f"unsupported Vifu feedback event: {event}")
        if outcome not in _OUTCOMES:
            raise ValueError(f"unsupported Vifu feedback outcome: {outcome}")

        payload: dict[str, str] = {"event": event, "outcome": outcome}
        if message:
            payload["message"] = _safe_text(message, _MAX_MESSAGE_LENGTH)
        if path:
            safe_path = _safe_text(path, _MAX_PATH_LENGTH)
            if not safe_path.startswith("$"):
                raise ValueError("feedback path must start with $")
            payload["path"] = safe_path
        body = json.dumps(payload).encode("utf-8")
        url = (
            f"{self._server_url}/{self._project_slug}/v1/traces/"
            f"{invocation_id}/feedback"
        )
        outgoing = request.Request(
            url,
            data=body,
            method="POST",
            headers={
                "Authorization": f"Bearer {self._project_key}",
                "Content-Type": "application/json",
            },
        )
        try:
            with _open_without_redirects(outgoing, self._timeout_seconds) as response:
                status_code = response.status
        except error.HTTPError as exc:
            raise RuntimeError(
                f"Vifu rejected {event} feedback with HTTP {exc.code}"
            ) from exc
        except error.URLError as exc:
            raise RuntimeError(f"Vifu feedback endpoint is unavailable: {exc.reason}") from exc
        return FeedbackResult(event=event, outcome=outcome, status_code=status_code)

    def output_accepted(
        self,
        invocation_id: str,
        parsed: Any,
        *,
        required_keys: Sequence[str] = (),
    ) -> FeedbackResult:
        if not isinstance(parsed, Mapping):
            return self.report(
                invocation_id,
                "OUTPUT_ACCEPTED",
                "fail",
                message="StarDojo parser did not produce an object",
                path="$",
            )
        for key in required_keys:
            if key not in parsed:
                return self.report(
                    invocation_id,
                    "OUTPUT_ACCEPTED",
                    "fail",
                    message=f"StarDojo response is missing required key {key}",
                    path=f"$.{key}",
                )
        return self.report(invocation_id, "OUTPUT_ACCEPTED", "pass")

    def parser_failed(
        self,
        invocation_id: str,
        exc: BaseException,
        *,
        path: str | None = None,
    ) -> FeedbackResult:
        return self.report(
            invocation_id,
            "OUTPUT_ACCEPTED",
            "fail",
            message=f"StarDojo parser rejected output ({type(exc).__name__})",
            path=path,
        )

    def action_applied(
        self, invocation_id: str, execution: Mapping[str, Any]
    ) -> FeedbackResult:
        error_value = execution.get("errors", execution.get("error"))
        message = execution.get("errors_info") or execution.get("error_info")
        executed = execution.get("executed_skills", execution.get("executedSkills"))
        if error_value is True:
            outcome = "fail"
        elif (
            error_value is False
            and isinstance(executed, Sequence)
            and not isinstance(executed, (str, bytes, bytearray))
            and len(executed) > 0
        ):
            outcome = "pass"
        else:
            outcome = "unknown"
        return self.report(
            invocation_id,
            "ACTION_APPLIED",
            outcome,
            message=str(message) if outcome == "fail" and message else None,
        )

    def frame_presented(
        self,
        invocation_id: str,
        screenshot_path: str | Path | None,
        *,
        presented: bool | None = None,
    ) -> FeedbackResult:
        if presented is None:
            return self.report(
                invocation_id,
                "FRAME_PRESENTED",
                "unknown",
                message="StarDojo did not verify the rendered frame",
            )
        if not presented:
            return self.report(
                invocation_id,
                "FRAME_PRESENTED",
                "fail",
                message="StarDojo frame verification failed",
            )
        if screenshot_path is None:
            return self.report(
                invocation_id,
                "FRAME_PRESENTED",
                "unknown",
                message="StarDojo did not return a screenshot path",
            )
        path = Path(screenshot_path)
        if not path.is_file() or path.stat().st_size == 0:
            return self.report(
                invocation_id,
                "FRAME_PRESENTED",
                "fail",
                message="StarDojo screenshot is missing or empty",
            )
        return self.report(invocation_id, "FRAME_PRESENTED", "pass")


class VifuFeedbackWorker:
    """Bounded background delivery so trace feedback never stalls the game loop."""

    def __init__(
        self,
        feedback: VifuFeedback,
        *,
        max_pending: int = 64,
        on_error: Callable[[Exception], None] | None = None,
    ) -> None:
        if max_pending <= 0:
            raise ValueError("max_pending must be greater than zero")
        self._feedback = feedback
        self._on_error = on_error
        self._queue: Queue[tuple[str, tuple[Any, ...], dict[str, Any]] | None] = Queue(
            maxsize=max_pending
        )
        self._thread = Thread(target=self._run, name="vifu-feedback", daemon=True)
        self._thread.start()

    def output_accepted(self, *args: Any, **kwargs: Any) -> bool:
        return self._submit("output_accepted", args, kwargs)

    def parser_failed(self, *args: Any, **kwargs: Any) -> bool:
        return self._submit("parser_failed", args, kwargs)

    def action_applied(self, *args: Any, **kwargs: Any) -> bool:
        return self._submit("action_applied", args, kwargs)

    def frame_presented(self, *args: Any, **kwargs: Any) -> bool:
        return self._submit("frame_presented", args, kwargs)

    def close(self, *, wait: bool = True) -> None:
        self._queue.put(None)
        if wait:
            self._thread.join()

    def _submit(self, method: str, args: tuple[Any, ...], kwargs: dict[str, Any]) -> bool:
        try:
            self._queue.put_nowait((method, args, kwargs))
        except Full:
            return False
        return True

    def _run(self) -> None:
        while True:
            item = self._queue.get()
            if item is None:
                return
            method, args, kwargs = item
            try:
                getattr(self._feedback, method)(*args, **kwargs)
            except Exception as exc:  # Telemetry must not crash StarDojo.
                if self._on_error is not None:
                    try:
                        self._on_error(exc)
                    except Exception:
                        pass


class _RejectRedirects(request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise error.HTTPError(req.full_url, code, "Vifu feedback redirect rejected", headers, fp)


def _open_without_redirects(outgoing: request.Request, timeout_seconds: float):
    return request.build_opener(_RejectRedirects).open(outgoing, timeout=timeout_seconds)


def _validated_server_url(value: str) -> str:
    parsed = parse.urlsplit(value.strip().rstrip("/"))
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        raise ValueError("server_url must be an HTTP or HTTPS URL")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("server_url must not contain credentials, query, or fragment")
    try:
        port = parsed.port
    except ValueError as exc:
        raise ValueError("server_url has an invalid port") from exc
    if parsed.scheme == "http" and not _is_loopback_host(parsed.hostname):
        raise ValueError("plaintext Vifu feedback is allowed only on loopback")
    netloc = parsed.hostname
    if ":" in netloc and not netloc.startswith("["):
        netloc = f"[{netloc}]"
    if port is not None:
        netloc = f"{netloc}:{port}"
    return parse.urlunsplit((parsed.scheme, netloc, parsed.path.rstrip("/"), "", ""))


def _is_loopback_host(host: str) -> bool:
    if host.casefold() == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def _safe_text(value: str, limit: int) -> str:
    normalized = " ".join(str(value).split())
    if not normalized:
        raise ValueError("feedback text must contain printable characters")
    return normalized[:limit]


def _canonical_uuid(value: str) -> str:
    try:
        return str(UUID(value.strip()))
    except (AttributeError, ValueError) as exc:
        raise ValueError("Vifu invocation id must be a UUID") from exc
