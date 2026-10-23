"""Data folders with onboarding done, reading John 3 in the KJV: seed.py <dir>..."""
import json
import pathlib
import sqlite3
import sys

for d in sys.argv[1:]:
    path = pathlib.Path(d)
    path.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(path / "study.sqlite")
    db.execute("CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL)")
    settings = {"onboarded": True, "translation": "kjv", "theme": "Light", "chapter": 999, "verse": 16}
    db.execute("INSERT INTO settings VALUES (?, ?)", ("app", json.dumps(settings)))
    db.commit()
    db.close()
