import json
import os
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parents[1]
cargo = shutil.which('cargo') or str(Path.home() / '.cargo/bin/cargo.exe')
metadata = json.loads(subprocess.check_output([cargo, 'metadata', '--locked', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc'], cwd=root))
licenses = root / 'THIRD-PARTY-LICENSES'
licenses.mkdir(exist_ok=True)
lines = ['Image Atlas third-party dependencies', '', 'Original app: Copyright (c) 2026 NIS, MIT', 'Dependency authors and licenses follow.', '']
for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
    if package['name'] == 'image-atlas':
        continue
    name = f"{package['name']}-{package['version']}"
    lines.extend([name, 'Authors: ' + '; '.join(package['authors']), 'License: ' + (package.get('license') or 'See included license file'), 'Source: ' + (package.get('repository') or package.get('source') or ''), ''])
    source = Path(package['manifest_path']).parent
    target = licenses / name
    for path in source.iterdir():
        if path.is_file() and path.name.upper().startswith(('LICENSE', 'COPYING', 'NOTICE', 'AUTHORS', 'COPYRIGHT', 'UNLICENSE')):
            target.mkdir(exist_ok=True)
            shutil.copy2(path, target / path.name)
    for relative in [package.get('license_file')]:
        if relative and (source / relative).is_file():
            target.mkdir(exist_ok=True)
            shutil.copy2(source / relative, target / Path(relative).name)
(root / 'ATTRIBUZIONI.txt').write_text('\n'.join(lines), encoding='utf-8')
print(f"Attributed {len(metadata['packages']) - 1} dependencies")
