"""Conservative source confinement guard, scoped to this production slice."""
from pathlib import Path
import hashlib
import json
import re

repo = Path.cwd()
ffi = repo / 'crates/dendro-windows-private-io/src/ffi.rs'
approved = '31b2d35de0f36b88d665ee03c3f19254a70e2f7dbcf5b68b1debe7af30147412'
assert hashlib.sha256(ffi.read_bytes()).hexdigest() == approved, 'ffi audit bytes changed'
files = list((repo / 'crates/dendro-platform-policy').rglob('*.rs'))
files += list((repo / 'crates/dendro-windows-private-io').rglob('*.rs'))
files += [repo / f'crates/dendro-core/src/platform_v1/{name}.rs' for name in ['mod', 'policy', 'windows']]
records = []
for path in sorted(files):
    text = path.read_text(encoding='utf-8')
    if path != ffi:
        # Deliberately conservative: strip comments, not code or string literals.
        code = re.sub(r'/\*.*?\*/|//[^\n]*', '', text, flags=re.S)
        assert not re.search(r'\bunsafe\b', code), f'unsafe outside ffi: {path}'
        assert not re.search(r'allow\s*\([^)]*unsafe', code), f'unsafe lint escape: {path}'
        assert not re.search(r'\binclude\s*!', code), f'unscanned include: {path}'
    records.append({'path': path.relative_to(repo).as_posix(),
                    'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
print(json.dumps({'status':'pass', 'sole_exception':ffi.relative_to(repo).as_posix(),
                  'ffi_sha256':approved, 'files': records}, indent=2))
