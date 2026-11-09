#!/usr/bin/env python3
"""Publish a tested Debian artifact using the workflow's repository token."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import tomllib
import urllib.request
import urllib.error
root = Path(__file__).resolve().parent.parent
version = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version']
repository = 'PaulAxer86/entanglo-linux'
base = f'https://api.github.com/repos/{repository}'
token = os.environ['GH_TOKEN']
def api(method, url, data=None, content_type='application/json'):
    request = urllib.request.Request(url, data=data, method=method, headers={
        'Authorization': f'Bearer {token}', 'Accept': 'application/vnd.github+json',
        'Content-Type': content_type, 'X-GitHub-Api-Version': '2022-11-28',
    })
    with urllib.request.urlopen(request, timeout=120) as response:
        return json.load(response)
try:
    release = api('GET', f'{base}/releases/tags/v{version}')
    print(f'Release v{version} already exists; bump Cargo.toml before publishing another release.')
    raise SystemExit(0)
except urllib.error.HTTPError as error:
    if error.code != 404: raise
package = root / f'dist/entanglo-linux_{version}_amd64.deb'
manifest = {
    'version': version, 'date': datetime.date.today().isoformat(),
    'title': f'Linux {version}: direct Ethernet, media keys and improved pointer control',
    'url': f'https://github.com/{repository}/releases/download/v{version}/{package.name}',
    'sha256': hashlib.sha256(package.read_bytes()).hexdigest(), 'size': package.stat().st_size,
    'minimumSystemVersion': 'Debian 12', 'architecture': 'amd64',
    'sourceCommit': os.environ['RELEASE_SHA'],
    'highlights': ['Automatically prefer a reachable Ethernet path for trusted peers, with Wi-Fi fallback.',
                   'Keep each Bonjour interface and address, including scoped link-local IPv6.',
                   'Use the selected connection for input, files and clipboard; show its actual interface.',
                   'Forward volume, brightness and playback keys; return the pointer at the adjoining edge.',
                   'Adjust remote pointer speed independently on each controller.',
                   'Clean up failed TCP sessions so automatic reconnect can recover.'],
}
manifest_path = root / 'dist/latest-linux.json'
manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
release = api('POST', f'{base}/releases', json.dumps({
    'tag_name': f'v{version}', 'target_commitish': os.environ['RELEASE_SHA'],
    'name': f'Entanglo Linux {version}', 'draft': True,
    'body': '\n'.join('- ' + item for item in manifest['highlights']) + '\n\nDebian 12 and 13 amd64, X11. Install the .deb, add your user to the input group, and trust your controller.',
}).encode())
for path in [package, manifest_path, root / 'dist/INSTALL.txt']:
    url = release['upload_url'].split('{')[0] + '?name=' + path.name
    api('POST', url, path.read_bytes(), 'application/octet-stream')
api('PATCH', f"{base}/releases/{release['id']}", json.dumps({'draft': False}).encode())
print(f'Published {release["html_url"]}')
