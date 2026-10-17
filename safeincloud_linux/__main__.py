import argparse
import getpass
import json
import sys

from . import DatabaseError, cards, load


def main():
    parser = argparse.ArgumentParser(description="Read-only SafeInCloud viewer (experimental)")
    parser.add_argument("database", nargs="?", help="Local COPY of an encrypted SafeInCloud .db file")
    parser.add_argument("--gui", action="store_true", help="Open a local desktop viewer")
    parser.add_argument("--show", metavar="ID", help="Explicitly reveal fields of one card")
    args = parser.parse_args()
    if args.gui or args.database is None:
        from .startup import launch
        return launch(args.database)
    try:
        root = load(args.database, getpass.getpass("Master password: "))
        entries = cards(root)
        if args.show is None:
            for card in entries:
                print(json.dumps({"id": card.get("id"), "title": card.get("title")}, ensure_ascii=True))
        else:
            selected = [c for c in entries if c.get("id") == args.show]
            if not selected:
                parser.exit(1, "Card not found\n")
            for card in selected:
                print(json.dumps({"title": card.get("title"), "fields": [
                    {"name": f.get("name"), "type": f.get("type"), "value": f.text or ""}
                    for f in card.findall("field")], "notes": card.findtext("notes", "")}, ensure_ascii=True))
    except (DatabaseError, OSError, EOFError, KeyboardInterrupt) as exc:
        print(f"Cannot open database: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
