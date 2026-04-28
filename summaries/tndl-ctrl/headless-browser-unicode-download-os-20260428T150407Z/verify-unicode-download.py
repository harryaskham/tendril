from pathlib import Path
import json, sys, time
artifact = Path(__file__).resolve().parent
filename = "café-π-✓-report.txt"
expected = "download unicode café π ✓ 😀\n"
downloads = Path.home() / "Downloads"
path = downloads / filename
for _ in range(80):
    if path.exists():
        break
    time.sleep(0.25)
result = {
    "home": str(Path.home()),
    "downloads": str(downloads),
    "expected_filename": filename,
    "path": str(path),
    "exists": path.exists(),
    "files": sorted(p.name for p in downloads.glob("*")) if downloads.exists() else [],
}
if path.exists():
    result["contents"] = path.read_text(encoding="utf-8")
    result["matches"] = result["contents"] == expected
else:
    result["contents"] = None
    result["matches"] = False
(artifact / "os-unicode-download-readback.json").write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
(artifact / "os-unicode-download-proof.txt").write_text("unicode-download-os-proof: "+("ok" if result["matches"] else "mismatch")+"\n", encoding="utf-8")
print(json.dumps(result, ensure_ascii=False))
sys.exit(0 if result["matches"] else 1)
