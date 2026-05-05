"""Error formatting for LLM-backend API failures.

Kept separate from the agent loop so the loop's main control flow stays
readable. ``format_api_error`` takes whatever the OpenAI/Anthropic SDK raised
and produces a single ``str`` with as much context as the SDK exposed.
"""

from __future__ import annotations


_EMPTY_BODY_HINT = (
    "empty response body — usually means a malformed/empty API key or auth "
    "header. Check ~/.socai/auth.json or run /model in the CLI to re-enter."
)


def format_api_error(exc: BaseException) -> str:
    parts: list[str] = [type(exc).__name__]
    base = str(exc).strip()
    if base:
        parts.append(base)

    status = getattr(exc, "status_code", None)
    if status is not None:
        parts.append(f"status={status}")

    request_id = getattr(exc, "request_id", None) or getattr(exc, "_request_id", None)
    if request_id:
        parts.append(f"request_id={request_id}")

    message_attr = getattr(exc, "message", None)
    if message_attr and message_attr != base:
        parts.append(str(message_attr))

    body = getattr(exc, "body", None)
    if isinstance(body, dict):
        err = body.get("error") if isinstance(body.get("error"), dict) else body
        if isinstance(err, dict):
            for key in ("message", "code", "type", "param"):
                value = err.get(key)
                if value:
                    parts.append(f"{key}={value}")
    elif body:
        parts.append(f"body={body}")

    response_text = ""
    response = getattr(exc, "response", None)
    if response is not None:
        try:
            response_text = response.text or ""
        except Exception:  # noqa: BLE001 - diagnostic only
            response_text = ""
        if response_text:
            snippet = response_text.strip()
            if len(snippet) > 500:
                snippet = snippet[:500] + "…"
            parts.append(f"response={snippet}")

    # Heuristic: 4xx with no body anywhere is almost always a Cloudflare-edge
    # rejection of a malformed Authorization header.
    if status in (400, 401, 403) and not body and not response_text:
        parts.append(_EMPTY_BODY_HINT)

    seen: set[str] = set()
    cleaned: list[str] = []
    for part in parts:
        if part and part not in seen:
            seen.add(part)
            cleaned.append(part)
    return " | ".join(cleaned)
