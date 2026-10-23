"""Lays out <out> like GitHub for <version>: latest.json (the API's answer, with sizes and SHA-256
digests) and releases/download/v<version>/ with the installer and checksum list from <dist>."""
import hashlib
import json
import pathlib
import shutil
import sys

dist, out, version = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3]
tag, base = f"v{version}", "http://127.0.0.1:8765"
files = out / "releases" / "download" / tag
files.mkdir(parents=True, exist_ok=True)
assets = []
for name in [f"bible-rs-{version}-windows-x64-setup.exe", f"bible-rs-{version}-windows-x64-SHA256SUMS.txt"]:
    shutil.copy(dist / name, files / name)
    data = (files / name).read_bytes()
    assets.append({
        "name": name,
        "browser_download_url": f"{base}/releases/download/{tag}/{name}",
        "size": len(data),
        "digest": "sha256:" + hashlib.sha256(data).hexdigest(),
    })
answer = {"tag_name": tag, "html_url": f"{base}/releases/tag/{tag}", "draft": False, "prerelease": False, "assets": assets}
(out / "latest.json").write_text(json.dumps(answer, indent=2))
print((out / "latest.json").read_text())
