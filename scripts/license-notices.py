"""Collect shipped dependency notices from installed Cargo/npm source packages."""
import json
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1', '--filter-platform', 'aarch64-apple-darwin'], cwd=root))
resolved = {item['id'] for item in metadata['resolve']['nodes']}
packages = [(p['name'], p['version'], p.get('license'), Path(p['manifest_path']).parent)
            for p in metadata['packages'] if p['id'] in resolved and p['source']]
lock = json.loads((root / 'web/package-lock.json').read_text())
for path, data in lock['packages'].items():
    if path and not data.get('dev'):
        packages.append((path.removeprefix('node_modules/'), data['version'], data.get('license'), root / 'web' / path))
parts = ['# Bundled dependency license notices\n\nGenerated from dependency sources used for this build. Some build/test dependency notices are included conservatively.\n']
for name, version, license_name, directory in sorted(packages):
    parts.append(f'\n## {name} {version}\nLicense expression: {license_name or "See package notice"}\n')
    notices = [p for p in directory.iterdir() if p.is_file() and p.name.upper().startswith(('LICENSE', 'COPYING', 'NOTICE', 'COPYRIGHT'))]
    for p in sorted(notices):
        parts.append(f'\n### {p.name}\n\n' + p.read_text(errors='replace') + '\n')
    if not notices:
        parts.append('\nPackage source: ' + (f'https://crates.io/crates/{name}/{version}' if 'registry' in str(directory) else f'https://www.npmjs.com/package/{name}/v/{version}') + '\n')
output = root / 'target/packages/DEPENDENCY-LICENSES.txt'
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text('\n'.join(parts))
print(output)
