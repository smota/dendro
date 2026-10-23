"""Reproducible scoped tests, py_compile, hashes, and a non-acceptance build receipt."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
OWNED = ['scripts/benchmark-v080.py'] + [str(p.relative_to(ROOT)).replace('\\', '/') for p in HERE.glob('*.py')] + [str((HERE / 'README.md').relative_to(ROOT)).replace('\\', '/')]
ORACLE = ROOT / '.agentflow/oracles/v080/T02-runner'


def sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as src:
        for block in iter(lambda: src.read(65536), b''):
            h.update(block)
    return h.hexdigest()


def call(argv, log=None):
    r = subprocess.run(argv, cwd=ROOT, shell=False, capture_output=True, timeout=60)
    if log:
        (HERE / log).write_bytes(r.stdout + r.stderr)
    return {'argv': argv, 'shell': False, 'exit_code': r.returncode,
            'stdout_sha256': hashlib.sha256(r.stdout).hexdigest(),
            'stderr_sha256': hashlib.sha256(r.stderr).hexdigest(),
            'log': log}


if __name__ == '__main__':
    test = call([sys.executable, '.agentflow/evidence/v080/benchmark-runner/test_benchmark_runner.py'], 'test-results.txt')
    sources = [ROOT / p for p in OWNED if p.endswith('.py')]
    # py_compile's explicit cfile prevents writes under scripts/__pycache__.
    compile_code = ('import pathlib,py_compile; base=pathlib.Path(' + repr(str(HERE)) + '); '
                    'paths=' + repr([str(p) for p in sources]) + '; '
                    '[(py_compile.compile(p,cfile=str(base/(pathlib.Path(p).name+".pyc")),doraise=True),'
                    '(base/(pathlib.Path(p).name+".pyc")).unlink()) for p in paths]')
    compiled = call([sys.executable, '-B', '-c', compile_code], 'py-compile-results.txt')
    help_check = call([sys.executable, 'scripts/benchmark-v080.py', '--help'], 'cli-help.txt')
    client = call(['codex', '--version'])
    oracle_names = ['BUILDER-CONTRACT.txt', 'frozen.json', 'record-key-table.json', 'freeze.json', 'golden-micro.jsonl']
    oracle_hashes = {name: sha(ORACLE / name) for name in oracle_names}
    if oracle_hashes['frozen.json'] != '63252603ac39789bfab0a6c93a4372215284c3220fdef5cd47d3a0a456f7908e':
        raise RuntimeError('Frozen oracle hash changed')
    if oracle_hashes['freeze.json'] != '9982ea1690212aa36ebfab3bb0d82cefcc63247980fe08c70b27f2a6855e8ea9':
        raise RuntimeError('Freeze receipt hash changed')
    if oracle_hashes['BUILDER-CONTRACT.txt'] != '8b680a9073d831de7d943924ec7c983e7a592cbaf403195b87e756d12f1c20ea':
        raise RuntimeError('Builder contract hash changed')
    if oracle_hashes['record-key-table.json'] != 'a541bfb6e189b84cc92ca5207fc53283090d29ad398834c3b9109782fcb1bf32':
        raise RuntimeError('Frozen key-table hash changed')
    if oracle_hashes['golden-micro.jsonl'] != 'a56bd9f760544c5426cbebf09d748512d6aaffd84d79b1469a961294f2451d88':
        raise RuntimeError('Golden micro hash changed')
    coverage = json.loads((HERE / 'case-coverage.json').read_text())
    previous_receipt = json.loads((HERE / 'history/pre-final-oracle-rebind/build-receipt.json').read_text())
    if sha(ROOT / 'scripts/benchmark-v080.py') != previous_receipt['source_hashes_sha256']['scripts/benchmark-v080.py']:
        raise RuntimeError('Product source changed during evidence-only rebind')
    binding_record = json.loads((ROOT / '.agentflow/runs/v080/T02-runner/review-and-rebind-pending.json').read_text())
    if sha(ROOT / binding_record['review']['path']) != binding_record['review']['sha256']:
        raise RuntimeError('Review binding mismatch')
    freeze = json.loads((ORACLE / 'freeze.json').read_text())
    if sha(ORACLE / freeze['amendment']['response_artifact']) != freeze['amendment']['response_sha256']:
        raise RuntimeError('Original-author response binding mismatch')
    receipt = {
        'schema': 'dendro.benchmark-runner-build-receipt/v1',
        'created_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'task': 'T02-runner', 'worker_role': 'bounded Codex builder',
        'revision': 'final-original-author-oracle-rebind',
        'oracle_rebind': {
            'binding_record_path': '.agentflow/runs/v080/T02-runner/review-and-rebind-pending.json',
            'binding_record_sha256': sha(ROOT / '.agentflow/runs/v080/T02-runner/review-and-rebind-pending.json'),
            'previous_receipt_path': '.agentflow/evidence/v080/benchmark-runner/history/pre-final-oracle-rebind/build-receipt.json',
            'previous_receipt_sha256': sha(HERE / 'history/pre-final-oracle-rebind/build-receipt.json'),
            'review_02_path': '.agentflow/runs/v080/T02-runner/review-02.json',
            'review_02_sha256': sha(ROOT / '.agentflow/runs/v080/T02-runner/review-02.json'),
            'review_scope': 'Computational and evaluator logic narrowly accepted; no actual native performance or collector proof',
            'author_amendment': json.loads((ORACLE / 'freeze.json').read_text())['amendment'],
            'author_response_sha256': sha(ORACLE / 'author-two-null-response.json'),
            'product_source_changed_this_rebind': False,
        },
        'review_addressed': {'path': '.agentflow/evidence/v080/benchmark-runner/review-01.json',
                             'sha256': sha(HERE / 'review-01.json'),
                             'findings': ['F1', 'F2', 'F3', 'F4', 'N1', 'N2']},
        'history_preserved_sha256': {str(p.relative_to(HERE)).replace('\\', '/'): sha(p)
                                    for p in sorted((HERE / 'history').rglob('*')) if p.is_file()},
        'scope': ['scripts/benchmark-v080.py', '.agentflow/evidence/v080/benchmark-runner/**'],
        'client_identity': {
            'provider': 'OpenAI', 'harness': 'Codex',
            'session_id_observed': os.environ.get('CODEX_SESSION_ID'),
            'thread_id_observed': os.environ.get('CODEX_THREAD_ID'),
            'client_version_observed': os.environ.get('CODEX_VERSION'),
            'client_executable_observed': os.environ.get('CODEX_CLI_PATH'),
            'originator_observed': os.environ.get('CODEX_INTERNAL_ORIGINATOR_OVERRIDE'),
            'model_execution_identity': 'not independently observed; no inferred model claim',
            'rig_whoami': {'command': 'rig whoami --json', 'exit_code': 1,
                           'observed_result': 'rig command not found; no OpenRig seat identity claimed'},
            'codex_version_check': client,
        },
        'host': {'os_family': platform.system().lower(), 'os_version': platform.release(),
                 'python_version': platform.python_version(), 'python_executable': sys.executable,
                 'native_linux_execution_performed': False},
        'commands': [test, compiled, help_check],
        'validation_successful': all(x['exit_code'] == 0 for x in (test, compiled, help_check)),
        'source_hashes_sha256': {p: sha(ROOT / p) for p in sorted(OWNED)},
        'oracle_hashes_sha256': oracle_hashes,
        'protected_files_current_hashes_only': {
            p: sha(ROOT / p) for p in ('scripts/portfolio-fixture.py', 'templates/nao/validate_performance.py')},
        'protected_files_written_by_this_worker': False,
        'coverage': {'oracle_cases': coverage['cases_total'],
                     'semantic_cases_exercised': len(coverage['covered_semantic_cases']),
                     'normative_semantic_cases_exercised': coverage['normative_semantic_cases_exercised'],
                     'pending_exact_field_amendments': coverage['pending_exact_field_amendments'],
                     'resolved_exact_field_amendments': coverage['resolved_exact_field_amendments'],
                     'unresolved_cases': coverage['unresolved_cases'],
                     'unaccounted_cases': coverage['unaccounted_cases'],
                     'unittest_tests_run': coverage['tests_run'],
                     'case_coverage_sha256': sha(HERE / 'case-coverage.json'),
                     'scope': '89 semantic cases; zero pending exact-field amendments; not native acceptance'},
        'synthetic_fixture': {'path': '.agentflow/evidence/v080/benchmark-runner/synthetic-four-run-fixture.json',
                              'sha256': sha(HERE / 'synthetic-four-run-fixture.json'),
                              'actual_native_measurements': False},
        'oracle_amendments_resolution_sha256': sha(HERE / 'oracle-amendments-pending.json'),
        'limitations': [
            'No native whole-tree collector implementation supplied; absent evidence remains null/blocked. Imported closed OS-compatible bound claims are evaluated, not certified true.',
            'No built-in independent memory/UI/decode collectors; required affected metrics stay null unless separate bound evidence is imported.',
            'Imported evidence validates schema, binding, method/source/scope labels; instrument truth and custody require independent review.',
            'No actual native Dendro Windows or Linux performance run; only synthetic helper subprocesses.',
            'Positive four-run synthetic computational full_acceptance is tested; independent_acceptance remains false with independent_review_required status.',
            'No comparable v070 baseline fabricated or assumed.',
        ],
        'product_measurements_performed': False, 'independent_acceptance': False,
        'self_acceptance': False, 'commits': False, 'pushes': False, 'installs': False,
        'oracle_edits': False, 'real_user_log_reads': False, 'nested_agents': False,
    }
    path = HERE / 'build-receipt.json'
    temp = HERE / '.build-receipt.tmp'
    with open(temp, 'w', encoding='utf-8', newline='\n') as out:
        json.dump(receipt, out, indent=2, sort_keys=True)
        out.write('\n')
        out.flush()
        os.fsync(out.fileno())
    os.replace(temp, path)
    print(json.dumps({'receipt': str(path.relative_to(ROOT)), 'validation_successful': receipt['validation_successful'],
                      'tests_run': coverage['tests_run'], 'semantic_cases_exercised': len(coverage['covered_semantic_cases']),
                      'normative_semantic_cases_exercised': coverage['normative_semantic_cases_exercised'],
                      'unresolved_cases': list(coverage['unresolved_cases']), 'independent_acceptance': False}))
    sys.exit(0 if receipt['validation_successful'] else 1)
