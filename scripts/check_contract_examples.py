"""Validate contract examples structurally; this is not a domain-state reducer test."""
import argparse
from copy import deepcopy
from datetime import datetime, timedelta
import json
from pathlib import Path
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dependency-dir", type=Path, help="Optional isolated jsonschema installation")
    args = parser.parse_args()
    if args.dependency_dir:
        sys.path.insert(0, str(args.dependency_dir.resolve(strict=True)))
    from jsonschema import Draft202012Validator, FormatChecker

    root = Path(__file__).resolve().parent.parent / "contracts/v1"
    schema = json.loads((root / "collector-event.schema.json").read_text(encoding="utf-8"))
    examples = json.loads((root / "examples.json").read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema, format_checker=FormatChecker())
    events = [case["event"] for case in examples["ordered_scenario"]]
    deliveries = [case["event"] for case in examples["delivery_cases"]]
    for event in events + deliveries:
        validator.validate(event)
        if event["observed_at"] is not None:
            stamp = datetime.fromisoformat(event["observed_at"])
            if stamp.utcoffset() != timedelta(0):
                raise ValueError("Examples must use UTC timestamps")
    if [event["source_sequence"] for event in events] != [str(i) for i in range(1, len(events) + 1)]:
        raise ValueError("Ordered scenario must have contiguous source sequences")
    if len({(e["capture_id"], e["source_sequence"]) for e in events}) != len(events):
        raise ValueError("Ordered scenario contains duplicate delivery keys")
    # Negative examples check structural boundaries; they do not simulate storage.
    negatives = []
    for field, value in (("contract_version", 2), ("source_sequence", 5),
                         ("source_sequence", "05"), ("capture_id", "not-a-uuid"),
                         ("kind", "unknown.event"), ("profile", None)):
        event = deepcopy(events[4])
        event[field] = value
        negatives.append(event)
    event = deepcopy(events[7])
    event["payload"]["operations"][0]["values"] = {"count": 0}
    negatives.append(event)
    for event in negatives:
        if validator.is_valid(event):
            raise ValueError("Invalid structural example was accepted")
    print(f"Contract schema valid; {len(events)} ordered + {len(deliveries)} delivery examples valid; "
          f"{len(negatives)} malformed envelopes rejected. Domain transition scenarios are specifications only.")


if __name__ == "__main__":
    main()
