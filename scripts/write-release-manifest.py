#!/usr/bin/env python3
import json
import pathlib
import re
import sys


def artifact_record(name: str, version: str):
    if name.endswith('.tar.gz'):
        kind = 'archive'
        fmt = 'tar.gz'
    elif name.endswith('.sha256'):
        kind = 'checksum'
        fmt = 'sha256'
    else:
        return None

    scope = 'source' if f'tendril-{version}-source.' in name else 'binary'
    record = {
        'name': name,
        'kind': kind,
        'format': fmt,
        'scope': scope,
    }

    if scope == 'binary':
        match = re.match(rf'^tendril-{re.escape(version)}-(.+)\.(?:tar\.gz|sha256)$', name)
        if match:
            record['system'] = match.group(1)

    return record


def main() -> int:
    if len(sys.argv) != 3:
        print('usage: scripts/write-release-manifest.py <tag> <artifact-dir>', file=sys.stderr)
        return 1

    tag = sys.argv[1]
    artifact_dir = pathlib.Path(sys.argv[2])
    version = tag[1:] if tag.startswith('v') else tag

    artifacts = []
    binary_systems = set()
    for path in sorted(artifact_dir.iterdir()):
        if not path.is_file() or path.name == 'release-manifest.json':
            continue
        record = artifact_record(path.name, version)
        if record is None:
            continue
        artifacts.append(record)
        if 'system' in record:
            binary_systems.add(record['system'])

    manifest = {
        'project': 'tendril',
        'version': version,
        'semver': version,
        'tag': tag,
        'binary_systems': sorted(binary_systems),
        'artifacts': artifacts,
    }

    json.dump(manifest, sys.stdout, indent=2)
    sys.stdout.write('\n')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
