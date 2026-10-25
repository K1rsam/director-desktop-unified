"""Freeze user-specified rule sources; never repair or overwrite their originals."""
from pathlib import Path
import argparse
import hashlib
import json
import shutil

ROOT = Path(__file__).resolve().parents[1]
PACKS = (
    ("series", "20261025-v6.7.2", Path("E:/Soullnk/.agents/skills/series-storyboard-director"),
     "20261025-v6.7.2-readable-image-bindings", "8a5fe9d55c0d40f5b8e82a4c9735201bb00415ea + local v6.7.2 amendments"),
    ("assets", "1.1.2", Path("E:/cx/资产项目/.agents/skills/gpt-image25-asset-workflow"), "V1.1.2", "user-installed V1.1.2"),
)

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def inventory(directory):
    return {p.relative_to(directory).as_posix(): {"sha256": sha(p), "bytes": p.stat().st_size}
            for p in sorted(directory.rglob("*")) if p.is_file()
            and not any(x in {"__pycache__", ".git", "node_modules", ".pytest_cache"} for x in p.parts)
            and p.name != "desktop-freeze.json" and p.suffix not in {".pyc", ".pyo"}}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    results = []
    for name, directory, source, version, origin in PACKS:
        target = ROOT / "rules" / name / directory
        manifest = target / "desktop-freeze.json"
        if args.verify:
            saved = json.loads(manifest.read_text(encoding="utf-8"))
            actual = inventory(target)
            if actual != saved["files"]:
                raise SystemExit(f"Frozen rule package changed: {name}")
        else:
            if target.exists():
                raise SystemExit(f"Refusing to replace an existing frozen package: {target}")
            files = inventory(source)
            for relative in files:
                dest = target / relative
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source / relative, dest)
            if inventory(target) != files:
                raise SystemExit(f"Copy verification failed: {name}")
            saved = {"schemaVersion": "desktop-rule-freeze/v1", "package": name,
                     "version": version, "sourceBasis": origin,
                     "upstreamMetadataPreserved": True,
                     "authority": "This manifest binds the copied bytes; upstream SHA256SUMS/provenance may predate local amendments.",
                     "files": files}
            manifest.write_text(json.dumps(saved, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        results.append({"package": name, "version": saved["version"], "files": len(saved["files"]),
                        "manifestSha256": sha(manifest), "verified": True})
    print(json.dumps(results, ensure_ascii=False, indent=2))

if __name__ == "__main__":
    main()
