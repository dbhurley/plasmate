from unittest.mock import Mock

from langchain_plasmate import PlasmateSOMLoader


def test_loader_forwards_selector() -> None:
    client = Mock()
    client.fetch_page.return_value = {
        "title": "Fixture",
        "url": "fixture",
        "regions": [],
        "meta": {
            "html_bytes": 0,
            "som_bytes": 0,
            "element_count": 0,
            "interactive_count": 0,
        },
    }
    loader = PlasmateSOMLoader(["fixture"], client=client, selector="main")

    documents = loader.load()

    assert len(documents) == 1
    client.fetch_page.assert_called_once_with("fixture", selector="main")
