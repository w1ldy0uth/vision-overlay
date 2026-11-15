# Collector contract v1

This directory defines the next application contract; current probe JSONL v1 is
a separate legacy format. The backend ingests this contract; the ordinary-user
relay wraps live probe journals with explicit legacy_capture_v1 value provenance.

- `collector-event.schema.json`: Draft 2020-12 structural validation.
- `examples.json`: synthetic ordered lifecycle, delivery cases and expected
  semantic outcomes. These contain no real profile data or authentication secrets.
- `../../docs/WEB_APP_CONTRACTS_V1.md`: identity, domain semantics and legacy mapping.
- `../../docs/WEB_APP_FRESHNESS_AND_DELIVERY.md`: transaction, freshness and SSE rules.
- `../../docs/ADR_0001_LOCAL_WEB_STACK.md`: implementation choices.

The docs directory may be ignored locally. This schema is intentionally narrower
than full domain validation: entity fields, integer ranges, epoch ownership,
coverage, previous-state checks and transitions need the implementation tests in
the following milestones. A valid example is not evidence that SQLite/SSE exists.

`identity.confirmed.evidence.region_source` also accepts `uid_prefix` for global
Genshin UIDs verified by the login adapter. The backend independently validates
the supported prefix/length against `profile.server_region`; the enum value alone
does not establish a valid binding. Explicit region mappings remain supported.

Reproduce structural checks using an isolated local dependency directory:

```powershell
python -m pip install --target local-data/contract-validation jsonschema==4.26.0
python scripts/check_contract_examples.py --dependency-dir local-data/contract-validation
```

`traveler_depots` stores observed complete element skill sets keyed by depot ID
within one profile. Partial snapshots preserve previously saved sets; a switch
updates this collection and the active character in one snapshot transaction.
