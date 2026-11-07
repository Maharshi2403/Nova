from harness_ai.providers.base import Usage, estimate_cost


def test_cached_tokens_priced_at_cached_rate() -> None:
    prices = {
        "input_usd_per_million_tokens": 1,
        "output_usd_per_million_tokens": 2,
        "cached_input_usd_per_million_tokens": 0.1,
        "cache_write_usd_per_million_tokens": 1.25,
    }
    usage = Usage(
        input_tokens=100, output_tokens=5, cached_input_tokens=20, cache_creation_input_tokens=10
    )
    cost = estimate_cost(usage, prices)
    assert cost is not None
    assert abs(cost - 0.0000945) < 1e-12


def test_missing_cache_price_remains_unknown() -> None:
    prices = {"input_usd_per_million_tokens": 1, "output_usd_per_million_tokens": 2}
    assert (
        estimate_cost(Usage(input_tokens=100, output_tokens=5, cached_input_tokens=20), prices)
        is None
    )
    assert (
        estimate_cost(
            Usage(input_tokens=100, output_tokens=5, cache_creation_input_tokens=20), prices
        )
        is None
    )
    assert estimate_cost(Usage(), prices) is None


def test_invalid_cache_totals_rejected() -> None:
    assert (
        estimate_cost(
            Usage(
                input_tokens=10,
                output_tokens=5,
                cached_input_tokens=9,
                cache_creation_input_tokens=2,
            ),
            {},
        )
        is None
    )
