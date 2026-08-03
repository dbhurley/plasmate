"""Tests for public SDK read-option forwarding."""

import asyncio
from unittest.mock import AsyncMock, Mock, call

from plasmate.client import AsyncPlasmate, Plasmate


def test_sync_read_methods_forward_selector() -> None:
    browser = Plasmate()
    calls = Mock(side_effect=[{}, "text"])
    browser._call_tool = calls  # type: ignore[method-assign]

    assert browser.fetch_page("fixture", selector="main") == {}
    assert browser.extract_text("fixture", selector="content") == "text"

    assert calls.call_args_list == [
        call("fetch_page", {"url": "fixture", "selector": "main"}),
        call("extract_text", {"url": "fixture", "selector": "content"}),
    ]


def test_async_read_methods_forward_selector() -> None:
    async def exercise() -> None:
        browser = AsyncPlasmate()
        calls = AsyncMock(side_effect=[{}, "text"])
        browser._call_tool = calls  # type: ignore[method-assign]

        assert await browser.fetch_page("fixture", selector="main") == {}
        assert await browser.extract_text("fixture", selector="content") == "text"

        assert calls.call_args_list == [
            call("fetch_page", {"url": "fixture", "selector": "main"}),
            call("extract_text", {"url": "fixture", "selector": "content"}),
        ]

    asyncio.run(exercise())
