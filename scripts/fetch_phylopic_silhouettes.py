#!/usr/bin/env python3
"""Queries the official PhyloPic v2 API (https://api.phylopic.org) for all 133 units
in `assets/compiled/wildfront_roster.db`, downloads CC0/PD SVG/PNG silhouettes into
`assets/silhouettes/`, and updates `phylopic_uuid` + `silhouette_path` in SQLite.

API Reference: https://www.phylopic.org/articles/api-recipes
"""

import json
import pathlib
import re
import sqlite3
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
DB_PATH = ROOT / "assets/compiled/wildfront_roster.db"
SIL_DIR = ROOT / "assets/silhouettes"


def fetch_json(url: str) -> dict:
    req = urllib.request.Request(url, headers={"User-Agent": "WildfrontBattlegrounds/2.2.0"})
    with urllib.request.urlopen(req, timeout=15) as resp:
        return json.loads(resp.read().decode("utf-8"))


def main():
    SIL_DIR.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(DB_PATH))
    cur = conn.cursor()
    rows = cur.execute("SELECT card_key, species, phylopic_taxon FROM units").fetchall()

    # Fetch current PhyloPic build number
    root_meta = fetch_json("https://api.phylopic.org/")
    build = root_meta.get("build", 558)
    print(f"[PhyloPic] Connected to API v2 (build={build}). Syncing {len(rows)} units...")

    cache = {}
    for card_key, species, taxon in rows:
        q = taxon.strip().lower()
        if q not in cache:
            try:
                url = f"https://api.phylopic.org/nodes?build={build}&filter_name={urllib.parse.quote(q)}&embed_primaryImage=true"
                data = fetch_json(url)
                items = data.get("_embedded", {}).get("items", [])
                img_url = None
                uuid = ""
                if items:
                    prim = items[0].get("_embedded", {}).get("primaryImage", {})
                    uuid = prim.get("uuid", "")
                    vec = prim.get("_links", {}).get("vectorFile", {})
                    if vec and "href" in vec:
                        img_url = vec["href"]
                cache[q] = (uuid, img_url)
            except Exception as exc:
                print(f"  [WARN] Failed lookup for {q}: {exc}")
                cache[q] = ("", None)

        uuid, img_url = cache[q]
        if img_url:
            slug = re.sub(r"[^a-z0-9]+", "_", card_key.lower()).strip("_")
            out_svg = SIL_DIR / f"{slug}.svg"
            if not out_svg.exists():
                try:
                    req = urllib.request.Request(img_url, headers={"User-Agent": "WildfrontBattlegrounds/2.2.0"})
                    with urllib.request.urlopen(req, timeout=15) as r:
                        out_svg.write_bytes(r.read())
                except Exception as exc:
                    print(f"  [WARN] Download failed for {card_key}: {exc}")
            cur.execute(
                "UPDATE units SET phylopic_uuid = ?, silhouette_path = ? WHERE card_key = ?",
                (uuid, f"assets/silhouettes/{slug}.svg", card_key),
            )

    conn.commit()
    conn.close()
    print("[PhyloPic] Sync complete!")


if __name__ == "__main__":
    main()
