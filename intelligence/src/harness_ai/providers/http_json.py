"""Bounded authenticated JSON transport; redirects and ambient proxies are disabled."""

from __future__ import annotations

import asyncio
import ipaddress
import json
import re
from typing import Any
from urllib.parse import urlsplit

import httpx

from harness_ai.providers.base import PermanentProviderError, TransientProviderError

_CODE = re.compile(r"[A-Za-z0-9_.\-]{1,64}")
ERROR_BODY_PEEK_BYTES = 4096


async def http_error(response: httpx.Response) -> str:
    """`provider HTTP <status>` plus the provider's error code/type identifier, if any.

    Only a short identifier such as `insufficient_quota` is kept; messages and bodies are
    never surfaced because they can echo prompts or account details.
    """
    detail = f"provider HTTP {response.status_code}"
    try:
        data = bytearray()
        async for chunk in response.aiter_bytes():
            data.extend(chunk)
            if len(data) >= ERROR_BODY_PEEK_BYTES:
                break
        error = json.loads(bytes(data)).get("error")
        candidates = [error.get("code"), error.get("type")] if isinstance(error, dict) else []
    except (ValueError, TypeError, AttributeError, httpx.HTTPError):
        candidates = []
    for value in candidates:
        if isinstance(value, str) and _CODE.fullmatch(value):
            return f"{detail} {value}"
    return detail


def validate_base(base: str) -> str:
    parsed = urlsplit(base)
    try:
        local = (
            parsed.hostname == "localhost"
            or ipaddress.ip_address(parsed.hostname or "").is_loopback
        )
    except ValueError:
        local = False
    if (
        not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.query
        or parsed.fragment
        or parsed.scheme not in ("https", "http")
        or (parsed.scheme == "http" and not local)
        or any(part in (".", "..") for part in parsed.path.split("/"))
    ):
        raise PermanentProviderError(
            "provider base URL must be HTTPS or loopback HTTP without credentials"
        )
    return base.rstrip("/")


async def request_json(
    method: str,
    url: str,
    *,
    headers: dict[str, str],
    body: dict[str, Any] | None = None,
    params: dict[str, str] | None = None,
    timeout: float = 30,
    limit: int = 2 * 1024 * 1024,
    transport: httpx.AsyncBaseTransport | None = None,
) -> dict[str, Any]:
    try:
        async with (
            asyncio.timeout(timeout),
            httpx.AsyncClient(
                trust_env=False, follow_redirects=False, timeout=timeout, transport=transport
            ) as client,
            client.stream(method, url, headers=headers, json=body, params=params) as response,
        ):
            if response.status_code == 429 or 500 <= response.status_code <= 599:
                raise TransientProviderError(await http_error(response))
            if response.status_code != 200:
                raise PermanentProviderError(await http_error(response))
            data = bytearray()
            async for chunk in response.aiter_bytes():
                if len(data) + len(chunk) > limit:
                    raise PermanentProviderError("provider response exceeds byte limit")
                data.extend(chunk)
    except (TimeoutError, httpx.TimeoutException, httpx.TransportError):
        raise TransientProviderError("provider transport failed or timed out") from None
    try:
        value = json.loads(data)
        if not isinstance(value, dict):
            raise ValueError
        return value
    except (ValueError, TypeError):
        raise PermanentProviderError("provider returned invalid JSON") from None
