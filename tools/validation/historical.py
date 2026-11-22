# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Run the native historical harness and preserve committed-source receipt checks."""
import argparse
from pathlib import Path
import subprocess
import sys

from verify import ROOT, require_clean_candidate, run

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--scratch', type=Path, required=True)
parser.add_argument('--target-dir', type=Path, required=True)
parser.add_argument('--offline', action='store_true')
args = parser.parse_args()
head = require_clean_candidate()
run(['git', 'cat-file', '-e', '6b755135f95e497071790e8e5e358995a8244a18^{commit}'])
command = [sys.executable, 'scripts/historical_migration_proof.py', '--repo', str(ROOT),
           '--scratch', str(args.scratch.resolve()), '--target-dir', str(args.target_dir.resolve())]
if args.offline:
    command.append('--offline')
run(command)
if subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip() != head:
    raise SystemExit('Candidate changed during historical acceptance')
require_clean_candidate()
receipts = args.scratch.resolve() / 'validation'
receipts.mkdir(mode=0o700)
run([sys.executable, 'tools/validation/validate_historical_receipt.py', str(ROOT),
     str(args.scratch.resolve() / 'receipt.json'), str(receipts), str(args.target_dir.resolve())])
