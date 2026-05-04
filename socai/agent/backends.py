"""LLM backend abstraction for the Socai agent loop.

The agent loop only depends on the small ``Backend`` interface in this module.
Hosted SDKs are imported lazily so the core can be tested without installing
provider packages.
"""

from __future__ import annotations

import json
import os
import re
import uuid
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from pathlib import Path


PROVIDER_ANTHROPIC = "anthropic"
PROVIDER_OPENAI = "openai"
PROVIDER_KIMI = "kimi"
PROVIDER_QWEN = "qwen"

SOCAI_AUTH_FILE = Path.home() / ".socai" / "auth.json"
CODEX_AUTH_FILE = Path.home() / ".codex" / "auth.json"


@dataclass(frozen=True)
class ProviderConfig:
    name: str
    display_name: str
    default_model: str
    api_key_env: tuple[str, ...]
    base_url: str | None = None
    model_prefixes: tuple[str, ...] = ()


PROVIDERS: dict[str, ProviderConfig] = {
    PROVIDER_ANTHROPIC: ProviderConfig(
        name=PROVIDER_ANTHROPIC,
        display_name="Anthropic",
        default_model="claude-sonnet-4-6",
        api_key_env=("ANTHROPIC_API_KEY",),
        model_prefixes=("claude-",),
    ),
    PROVIDER_OPENAI: ProviderConfig(
        name=PROVIDER_OPENAI,
        display_name="OpenAI",
        default_model="gpt-5.5",
        api_key_env=("OPENAI_API_KEY",),
        model_prefixes=("gpt-", "o1", "o3", "o4", "chatgpt-"),
    ),
    PROVIDER_KIMI: ProviderConfig(
        name=PROVIDER_KIMI,
        display_name="Kimi",
        default_model="kimi-k2.6",
        api_key_env=("KIMI_API_KEY", "MOONSHOT_API_KEY"),
        base_url="https://api.moonshot.cn/v1",
        model_prefixes=("kimi-", "moonshot-"),
    ),
    PROVIDER_QWEN: ProviderConfig(
        name=PROVIDER_QWEN,
        display_name="Qwen",
        default_model="qwen3.6-plus",
        api_key_env=("QWEN_API_KEY", "DASHSCOPE_API_KEY"),
        base_url="https://dashscope.aliyuncs.com/compatible-mode/v1",
        model_prefixes=("qwen", "qwq-", "qvq-"),
    ),
}


@dataclass
class ToolCall:
    """A parsed tool call from an LLM response."""

    id: str
    name: str
    input: dict


@dataclass
class LLMResponse:
    """Normalized response from any backend."""

    text_blocks: list[str]
    tool_calls: list[ToolCall]
    stop_reason: str
    input_tokens: int = 0
    output_tokens: int = 0
    raw: object = None
    metrics: dict = field(default_factory=dict)


_ASSISTANT_TEXT_MAX_CHARS = 320
_TOOL_RESULT_TEXT_MAX_CHARS = 2200


def default_model_for_provider(provider: str) -> str:
    config = PROVIDERS.get(provider)
    if config is None:
        raise ValueError(f"Unknown provider: {provider!r}")
    return _configured_default_model(provider) or config.default_model


def _read_json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return {}
    return value if isinstance(value, dict) else {}


def _auth_configs() -> list[tuple[Path, dict]]:
    configs: list[tuple[Path, dict]] = []
    data = _read_json(SOCAI_AUTH_FILE)
    if data:
        configs.append((SOCAI_AUTH_FILE, data))
    return configs


def save_api_key(provider: str, api_key: str) -> Path:
    provider = str(provider or "").strip().lower()
    if provider not in PROVIDERS:
        raise ValueError(f"Unknown provider: {provider!r}")
    secret = str(api_key or "").strip()
    if not secret:
        raise ValueError("API key is required.")

    data = _read_json(SOCAI_AUTH_FILE)
    provider_block = dict(data.get(provider) or {})
    provider_block["api_key"] = secret
    data[provider] = provider_block
    defaults = dict(data.get("defaults") or {})
    defaults["provider"] = provider
    data["defaults"] = defaults

    SOCAI_AUTH_FILE.parent.mkdir(parents=True, exist_ok=True)
    SOCAI_AUTH_FILE.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
    try:
        os.chmod(SOCAI_AUTH_FILE, 0o600)
    except OSError:
        pass
    return SOCAI_AUTH_FILE


def _configured_secret(provider: str, key: str) -> tuple[str, str] | None:
    for path, data in _auth_configs():
        block = data.get(provider) or {}
        if not isinstance(block, dict):
            continue
        value = str(block.get(key) or "").strip()
        if value:
            return str(path), value
    return None


def _configured_default_provider() -> str:
    for _, data in _auth_configs():
        defaults = data.get("defaults") or {}
        if not isinstance(defaults, dict):
            continue
        provider = str(defaults.get("provider") or "").strip().lower()
        if provider in PROVIDERS:
            return provider
    return ""


def _configured_default_model(provider: str) -> str:
    for _, data in _auth_configs():
        defaults = data.get("defaults") or {}
        if not isinstance(defaults, dict):
            continue
        model = str(defaults.get(f"{provider}_model") or "").strip()
        if model:
            return model
    return ""


def _codex_api_key() -> str:
    return str(_read_json(CODEX_AUTH_FILE).get("OPENAI_API_KEY") or "").strip()


def _provider_has_key(provider: str) -> bool:
    config = PROVIDERS.get(provider)
    if config is None:
        return False
    if any(os.environ.get(key, "").strip() for key in config.api_key_env):
        return True
    if _configured_secret(provider, "api_key"):
        return True
    return provider == PROVIDER_OPENAI and bool(_codex_api_key())


def has_any_api_key() -> bool:
    return any(_provider_has_key(provider) for provider in PROVIDERS)


def resolve_model_provider(model: str | None = None, provider: str | None = None) -> str:
    explicit = str(provider or os.environ.get("SOCAI_LLM_PROVIDER", "")).strip().lower()
    if explicit:
        if explicit not in PROVIDERS:
            raise ValueError(f"Unknown provider: {explicit!r}")
        return explicit

    normalized = str(model or os.environ.get("SOCAI_MODEL", "")).strip().lower()
    for name, config in PROVIDERS.items():
        if normalized and any(normalized.startswith(prefix) for prefix in config.model_prefixes):
            return name
    if configured := _configured_default_provider():
        return configured
    for name in PROVIDERS:
        if _provider_has_key(name):
            return name
    return PROVIDER_OPENAI


def _api_key_for(config: ProviderConfig) -> str:
    for key in config.api_key_env:
        value = os.environ.get(key, "").strip()
        if value:
            return value
    configured = _configured_secret(config.name, "api_key")
    if configured is not None:
        return configured[1]
    if config.name == PROVIDER_OPENAI:
        codex_key = _codex_api_key()
        if codex_key:
            return codex_key
    hint = " or ".join(f"${key}" for key in config.api_key_env)
    raise RuntimeError(
        f"No API key found for {config.display_name}. Set {hint}, "
        f"or add {config.name}.api_key to {SOCAI_AUTH_FILE}."
    )


def _truncate(text: str, max_chars: int) -> str:
    value = str(text or "").strip()
    if len(value) <= max_chars:
        return value
    return value[:max_chars] + "... [truncated]"


def _compact_json_value(value):
    if isinstance(value, dict):
        preferred_order = [
            "ok",
            "error",
            "message",
            "site",
            "action",
            "entity_type",
            "query",
            "count",
            "state",
            "result",
            "cards",
            "entity",
            "title",
            "url",
            "summary",
        ]
        keys = [key for key in preferred_order if key in value]
        keys.extend(key for key in value if key not in keys)
        return {key: _compact_json_value(value[key]) for key in keys[:16]}
    if isinstance(value, list):
        return [_compact_json_value(item) for item in value[:5]]
    if isinstance(value, str):
        return _truncate(value, 320)
    return value


def _compress_text_maybe_json(text: str, max_chars: int = _TOOL_RESULT_TEXT_MAX_CHARS) -> str:
    if len(text) <= max_chars:
        return text
    try:
        value = json.loads(text)
    except Exception:
        return text[:max_chars] + "\n... [truncated]"

    compact_text = json.dumps(_compact_json_value(value), ensure_ascii=False, indent=2)
    if len(compact_text) <= max_chars:
        return compact_text
    return compact_text[:max_chars] + "\n... [truncated]"


def _screenshot_hint_from_text(text: str) -> str | None:
    match = re.search(r"Screenshot saved to ([^\s]+)", text or "")
    return match.group(1) if match else None


def _summarize_result_blocks_for_history(
    blocks: list[dict],
    *,
    max_chars: int = _TOOL_RESULT_TEXT_MAX_CHARS,
) -> list[dict]:
    parts: list[str] = []
    screenshot_file = None

    for block in blocks:
        if not isinstance(block, dict):
            continue
        if block.get("type") == "text":
            text = str(block.get("text", ""))
            if not screenshot_file:
                screenshot_file = _screenshot_hint_from_text(text)
            parts.append(_compress_text_maybe_json(text, max_chars=max_chars))
        elif block.get("type") == "image":
            if screenshot_file:
                parts.append(f"[Image omitted from history. Screenshot file: {screenshot_file}.]")
            else:
                parts.append("[Image omitted from history.]")

    combined = "\n\n".join(part for part in parts if part).strip()
    if len(combined) > max_chars:
        combined = _compress_text_maybe_json(combined, max_chars=max_chars)
    return [{"type": "text", "text": combined or "(empty result)"}]


class Backend(ABC):
    """Abstract LLM backend for the agent loop."""

    @abstractmethod
    def create_message(
        self,
        *,
        system: str,
        messages: list[dict],
        tools: list[dict],
        max_tokens: int = 8192,
    ) -> LLMResponse:
        """Send a message and return a normalized response."""
        ...

    @abstractmethod
    def format_assistant_content(self, response: LLMResponse) -> object:
        """Format assistant output for appending to loop history."""
        ...

    @abstractmethod
    def format_tool_results(self, tool_calls: list[ToolCall], results: list[list[dict]]) -> dict:
        """Format tool results for appending to loop history."""
        ...


class AnthropicBackend(Backend):
    """Backend using Anthropic Messages API."""

    def __init__(self, model: str | None = None):
        import anthropic

        config = PROVIDERS[PROVIDER_ANTHROPIC]
        self.model = model or config.default_model
        self.client = anthropic.Anthropic(api_key=_api_key_for(config))

    def create_message(
        self,
        *,
        system: str,
        messages: list[dict],
        tools: list[dict],
        max_tokens: int = 8192,
    ) -> LLMResponse:
        response = self.client.messages.create(
            model=self.model,
            max_tokens=max_tokens,
            system=system,
            tools=tools,
            messages=messages,
        )

        text_blocks: list[str] = []
        tool_calls: list[ToolCall] = []
        for block in response.content:
            if block.type == "text":
                text_blocks.append(block.text)
            elif block.type == "tool_use":
                tool_calls.append(ToolCall(id=block.id, name=block.name, input=block.input))

        return LLMResponse(
            text_blocks=text_blocks,
            tool_calls=tool_calls,
            stop_reason=response.stop_reason,
            input_tokens=response.usage.input_tokens,
            output_tokens=response.usage.output_tokens,
            raw=response,
        )

    def format_assistant_content(self, response: LLMResponse) -> object:
        content: list[dict] = []
        for block in response.raw.content:
            if block.type == "text":
                text = _truncate(block.text, _ASSISTANT_TEXT_MAX_CHARS)
                if text:
                    content.append({"type": "text", "text": text})
            elif block.type == "tool_use":
                content.append({"type": "tool_use", "id": block.id, "name": block.name, "input": block.input})
        return content

    def format_tool_results(self, tool_calls: list[ToolCall], results: list[list[dict]]) -> dict:
        content = []
        for tc, result_blocks in zip(tool_calls, results):
            content.append(
                {
                    "type": "tool_result",
                    "tool_use_id": tc.id,
                    "content": _summarize_result_blocks_for_history(result_blocks),
                }
            )
        return {"role": "user", "content": content}


class OpenAICompatibleBackend(Backend):
    """Backend for OpenAI and OpenAI-compatible chat-completions providers."""

    PROVIDER = PROVIDER_OPENAI
    PRESERVE_REASONING_CONTENT = False

    def __init__(self, model: str | None = None):
        from openai import OpenAI

        config = PROVIDERS[self.PROVIDER]
        self.model = model or config.default_model
        kwargs: dict = {"api_key": _api_key_for(config)}
        if config.base_url:
            kwargs["base_url"] = config.base_url
        self.client = OpenAI(**kwargs)

    @staticmethod
    def _tool_to_schema(tool: dict) -> dict:
        return {
            "type": "function",
            "function": {
                "name": tool["name"],
                "description": tool.get("description", ""),
                "parameters": tool.get("input_schema") or {"type": "object", "properties": {}},
            },
        }

    @staticmethod
    def _blocks_to_text(blocks: list[dict]) -> str:
        parts: list[str] = []
        for block in blocks:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "text":
                parts.append(str(block.get("text", "")))
            elif block.get("type") == "image":
                parts.append("[image omitted]")
        return "\n\n".join(part for part in parts if part).strip() or "(empty)"

    @staticmethod
    def _message_extra_value(message: object, key: str) -> object:
        value = getattr(message, key, None)
        if value is not None:
            return value
        extra = getattr(message, "model_extra", None)
        if isinstance(extra, dict):
            return extra.get(key)
        return None

    def _request_extra_body(self, *, has_tools: bool) -> dict:
        return {}

    def _message_to_chat(self, message: dict) -> list[dict]:
        role = str(message.get("role") or "user")
        content = message.get("content")

        if role == "assistant":
            text_parts: list[str] = []
            tool_calls: list[dict] = []
            reasoning_content: str | None = None
            if isinstance(content, list):
                for item in content:
                    if not isinstance(item, dict):
                        continue
                    if item.get("type") == "text":
                        text_parts.append(str(item.get("text", "")))
                    elif item.get("type") == "reasoning_content":
                        reasoning_content = str(item.get("text") or "")
                    elif item.get("type") == "tool_use":
                        tool_calls.append(
                            {
                                "id": str(item.get("id") or ""),
                                "type": "function",
                                "function": {
                                    "name": str(item.get("name") or ""),
                                    "arguments": json.dumps(item.get("input") or {}, ensure_ascii=False),
                                },
                            }
                        )
            elif isinstance(content, str):
                text_parts.append(content)

            chat: dict = {"role": "assistant", "content": "\n".join(text_parts).strip() or None}
            if tool_calls:
                chat["tool_calls"] = tool_calls
            if self.PRESERVE_REASONING_CONTENT and tool_calls:
                chat["reasoning_content"] = reasoning_content or ""
            return [chat]

        if role == "user":
            if isinstance(content, str):
                text = content.strip()
                return [{"role": "user", "content": text}] if text else []
            if isinstance(content, list):
                result: list[dict] = []
                text_parts: list[str] = []
                for item in content:
                    if not isinstance(item, dict):
                        continue
                    if item.get("type") == "tool_result":
                        blocks = item.get("content") or []
                        if not isinstance(blocks, list):
                            blocks = [{"type": "text", "text": str(blocks)}]
                        result.append(
                            {
                                "role": "tool",
                                "tool_call_id": str(item.get("tool_use_id") or ""),
                                "content": self._blocks_to_text(blocks),
                            }
                        )
                    elif item.get("type") == "text":
                        text_parts.append(str(item.get("text", "")))
                joined = "\n".join(part for part in text_parts if part).strip()
                if joined:
                    result.append({"role": "user", "content": joined})
                return result

        return []

    def create_message(
        self,
        *,
        system: str,
        messages: list[dict],
        tools: list[dict],
        max_tokens: int = 8192,
    ) -> LLMResponse:
        chat_messages: list[dict] = [{"role": "system", "content": system}]
        for message in messages:
            chat_messages.extend(self._message_to_chat(message))

        chat_tools = [self._tool_to_schema(tool) for tool in tools]
        request: dict = {
            "model": self.model,
            "messages": chat_messages,
            "max_tokens": max_tokens,
        }
        if chat_tools:
            request["tools"] = chat_tools
            request["tool_choice"] = "auto"
        extra_body = self._request_extra_body(has_tools=bool(chat_tools))
        if extra_body:
            request["extra_body"] = extra_body

        response = self.client.chat.completions.create(**request)
        choice = response.choices[0]
        message = choice.message
        reasoning_content = self._message_extra_value(message, "reasoning_content")

        text_blocks = [str(message.content)] if getattr(message, "content", None) else []
        tool_calls: list[ToolCall] = []
        for tc in getattr(message, "tool_calls", None) or []:
            fn = getattr(tc, "function", None)
            raw_args = str(getattr(fn, "arguments", "") or "{}")
            try:
                parsed_args = json.loads(raw_args)
            except json.JSONDecodeError:
                parsed_args = {}
            tool_calls.append(
                ToolCall(
                    id=str(getattr(tc, "id", "") or uuid.uuid4().hex),
                    name=str(getattr(fn, "name", "") or ""),
                    input=parsed_args if isinstance(parsed_args, dict) else {},
                )
            )

        finish = str(choice.finish_reason or "")
        if finish == "tool_calls":
            stop_reason = "tool_use"
        elif finish == "length":
            stop_reason = "max_tokens"
        else:
            stop_reason = "end_turn"

        usage = getattr(response, "usage", None)
        return LLMResponse(
            text_blocks=text_blocks,
            tool_calls=tool_calls,
            stop_reason=stop_reason,
            input_tokens=int(getattr(usage, "prompt_tokens", 0) or 0),
            output_tokens=int(getattr(usage, "completion_tokens", 0) or 0),
            raw={"response": response, "reasoning_content": reasoning_content},
        )

    def format_assistant_content(self, response: LLMResponse) -> object:
        content: list[dict] = []
        raw = response.raw if isinstance(response.raw, dict) else {}
        if self.PRESERVE_REASONING_CONTENT and raw.get("reasoning_content") is not None:
            content.append({"type": "reasoning_content", "text": str(raw["reasoning_content"] or "")})
        for text in response.text_blocks:
            truncated = _truncate(text, _ASSISTANT_TEXT_MAX_CHARS)
            if truncated:
                content.append({"type": "text", "text": truncated})
        for tc in response.tool_calls:
            content.append({"type": "tool_use", "id": tc.id, "name": tc.name, "input": tc.input})
        return content

    def format_tool_results(self, tool_calls: list[ToolCall], results: list[list[dict]]) -> dict:
        content = []
        for tc, result_blocks in zip(tool_calls, results):
            content.append(
                {
                    "type": "tool_result",
                    "tool_use_id": tc.id,
                    "content": _summarize_result_blocks_for_history(result_blocks),
                }
            )
        return {"role": "user", "content": content}


class OpenAIBackend(OpenAICompatibleBackend):
    PROVIDER = PROVIDER_OPENAI


class KimiBackend(OpenAICompatibleBackend):
    PROVIDER = PROVIDER_KIMI
    PRESERVE_REASONING_CONTENT = True

    def _request_extra_body(self, *, has_tools: bool) -> dict:
        if not has_tools:
            return {}
        if self.model.startswith("kimi-k2.6"):
            return {"thinking": {"type": "disabled"}}
        return {}


class QwenBackend(OpenAICompatibleBackend):
    PROVIDER = PROVIDER_QWEN
    PRESERVE_REASONING_CONTENT = True

    def _request_extra_body(self, *, has_tools: bool) -> dict:
        return {"enable_thinking": False} if has_tools else {}


def create_backend(model: str | None = None, *, provider: str | None = None) -> Backend:
    """Create a backend from a model id or explicit provider.

    ``SOCAI_LLM_PROVIDER`` and ``SOCAI_MODEL`` are honored when arguments are
    omitted. Provider SDKs are imported only by the selected backend.
    """

    resolved_provider = resolve_model_provider(model, provider)
    selected_model = str(model or os.environ.get("SOCAI_MODEL", "")).strip() or default_model_for_provider(
        resolved_provider
    )

    if resolved_provider == PROVIDER_ANTHROPIC:
        return AnthropicBackend(model=selected_model)
    if resolved_provider == PROVIDER_KIMI:
        return KimiBackend(model=selected_model)
    if resolved_provider == PROVIDER_QWEN:
        return QwenBackend(model=selected_model)
    return OpenAIBackend(model=selected_model)
