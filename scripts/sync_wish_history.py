"""Private subprocess entry point for automatic sync. Never emits cache credentials."""
import argparse
import json
from pathlib import Path
import import_wish_history as wish


def synchronize(uid, region, output):
    candidates, _ = wish.discover(wish.find_game_data(), region)
    if not candidates:
        raise wish.ImportFailure("No matching cached history URL")
    candidate, (first_banner, first_page) = wish.select_access(candidates, uid, region, progress=lambda _: None)
    def request(context, banner, page, cursor):
        if (banner, page, cursor) == (first_banner, 1, "0"):
            return first_page
        return wish.request_page(context, banner, page, cursor)
    data = wish.fetch(candidate, uid, region, request=request, progress=lambda _: None)
    with output.open("x", encoding="utf-8") as stream:
        json.dump(data, stream, ensure_ascii=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uid", required=True)
    parser.add_argument("--region", required=True, choices=wish.REGIONS)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--interface-index", type=int)
    args = parser.parse_args()
    status = "failed"
    try:
        if args.interface_index is not None:
            if not 1 <= args.interface_index <= 0xffffff:
                raise ValueError("Invalid interface")
            wish._interface_index = args.interface_index
        synchronize(args.uid, args.region, args.directory / "history.json")
        status = "success"
    except wish.ApiFailure as error:
        status = "needs_access" if error.code in (-100, -101) else "failed"
    except wish.ImportFailure as error:
        # Only map our own known safe failure messages; never echo exceptions.
        if any(token in str(error) for token in ("cached", "Cached", "cache", "Game log", "Game data", "No valid")):
            status = "needs_access"
    except Exception:
        pass
    (args.directory / "result.json").write_text(json.dumps({"status": status}), encoding="utf-8")


if __name__ == "__main__":
    main()
