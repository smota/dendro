"""Semantic tests: frozen independent inputs/expected values, real runner functions.

Coverage IDs annotate tests; they never select implementation return values.
"""
import collections
import copy
import importlib.util
import itertools
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True  # Imported runner must not write outside owned evidence.
ROOT = Path(__file__).resolve().parents[4]
ORACLE = ROOT / '.agentflow/oracles/v080/T02-runner'
SPEC = importlib.util.spec_from_file_location('benchmark_v080', ROOT / 'scripts/benchmark-v080.py')
b = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(b)
FROZEN = json.loads((ORACLE / 'frozen.json').read_text())
CASES = {c['id']: c for kind in ('measurement_correctness_testcases', 'failure_semantics_testcases') for c in FROZEN[kind]}
COVERAGE = {}


def covers(*ids):
    def decorate(fn):
        for case in ids:
            assert case in CASES
            COVERAGE[case] = fn.__name__
        return fn
    return decorate


def manifest(metrics=('warm_answer_p95_ns',)):
    m = {k: 'a' * 64 for k in b.MANIFEST_FIELDS if k.endswith('sha256')}
    m.update(manifest_version=b.MANIFEST, os_id='linux', compiler_id='fixture-cc-1',
             storage_id='fixture-disk-1', command_surface='v080', shell=False,
             seed=b.SEED, profile='micro', corpus_version=b.CORPUS,
             max_capture_bytes=65536, timeout_ns=1000000000,
             required_metrics=list(metrics), clock='monotonic')
    return m


def report(metrics=('warm_answer_p95_ns',)):
    return b.empty_report(manifest(metrics), {'os_family': 'linux', 'os_version': 'fixture-1',
                          'environment': 'native', 'binary_format': 'ELF'})


def sample(mode, ns, **kwargs):
    s = dict(mode=mode, phase='warmup' if mode == 'warmup' else 'measured',
             candidate_exit_code=0, candidate_exit_method='subprocess_returncode',
             termination_reason='none', elapsed_ns=ns, record_count=82,
             answer_count=7 if mode == 'seven_answer_home' else 1,
             capture_bytes=200, response_schema_valid=True, clock='monotonic',
             method='monotonic_elapsed', source='harness.monotonic_ns', scope='candidate_process',
             measurement_label='process_warm_os_cache' if mode in ('warmup', 'warm_answer') else 'process_elapsed')
    s.update(kwargs)
    return s


def warm_report():
    r = report()
    r['samples'] = [sample('warmup', 900000000), sample('warmup', 800000000)] + [sample('warm_answer', 1) for _ in range(20)]
    return r


def request():
    return dict(protocol=b.ANSWER, corpus_id=b.corpus_id('micro'),
                question_id='attempt-micro-8675309-00000', mode='warm_answer')


def descendant_evidence(r, count=0):
    family = r['native']['os_family']
    method = 'windows_job_object' if family == 'windows' else 'linux_cgroup_v2'
    source = 'win32.JobObject.accounting.all_descendants' if family == 'windows' else 'linux.cgroup_v2.accounting.all_descendants'
    return dict(candidate_binding=r['candidate_binding'], observation=dict(
        descendant_count=count, method=method, source=source,
        scope='process_tree', collector_verified=True))


def synthetic_four_runs():
    """Synthetic report claims at frozen caps. No native execution is asserted."""
    cases = CASES['full_acceptance.four_runs']['input']['runs']
    reports = []
    for frozen in cases:
        p, family = frozen['profile'], frozen['native'].split('_')[0]
        required = ['cold_migration', 'process_tree']
        required += ['warm_answer_p95_ns', 'warm_throughput', 'seven_answer_home_ns', 'ui_blocking_ns', 'small_process_resident_bytes'] if p == 'small' else ['large_reliable_peak_rss_bytes', 'retained_context_records', 'max_retained_fact_payload_slots']
        r = report(required)
        r['manifest'].update(profile=p, os_id=family, corpus_sha256=('b' if p == 'small' else 'c') * 64,
                             candidate_binary_sha256=('d' if family == 'windows' else 'e') * 64)
        r['candidate_binding'] = b.binding_for(r['manifest'])
        r['native'].update(os_family=family, binary_format='PE' if family == 'windows' else 'ELF')
        r['samples'] = [sample('cold_migration', ns) for ns in (500, 100, 400, 200, 300)]
        r['instrumentation']['descendants'] = descendant_evidence(r)
        if p == 'small':
            r['samples'] += [sample('warmup', 900000000)] * 2 + [sample('warm_answer', frozen['warm_p95_ns']) for _ in range(20)]
            r['samples'].append(sample('seven_answer_home', frozen['seven_answer_home_ns']))
            r['instrumentation']['ui_trace'] = dict(candidate_binding=r['candidate_binding'], method='ui_event_loop_trace', clock='monotonic', samples_ns=[frozen['ui_blocking_ns']])
            r['instrumentation']['small_memory'] = dict(candidate_binding=r['candidate_binding'], observation=dict(
                value=frozen['small_resident_bytes'], method='sampled_maximum', scope='candidate_process',
                source='win32.GetProcessMemoryInfo.WorkingSetSize' if family == 'windows' else 'procfs.status.VmRSS'))
        else:
            obs = dict(value=frozen['large_peak_rss_bytes'], method='reliable_peak', scope='candidate_process',
                       source='win32.GetProcessMemoryInfo.PeakWorkingSetSize' if family == 'windows' else 'posix.wait_rusage.ru_maxrss')
            obs['process_handle_held_until_after_exit' if family == 'windows' else 'collector_verified'] = True
            r['instrumentation']['large_memory'] = dict(candidate_binding=r['candidate_binding'], observation=obs)
            r['instrumentation']['decode'] = dict(candidate_binding=r['candidate_binding'], method='independent_decode_counter', scope='candidate_process', source='independent_decode_trace',
                retained_context_records=frozen['retained_context_records'], max_retained_fact_payload_slots=1,
                incremental_decode_records=1000, prefix_validation_bytes=64, payload_io_bytes=4096)
        r['evaluation'] = b.evaluate(r)
        reports.append(r)
    return reports


def rebind(r):
    """Keep each modified synthetic claim internally bound, exposing cross-run checks."""
    r['candidate_binding'] = b.binding_for(r['manifest'])
    for obs in r['instrumentation'].values():
        if obs is not None:
            obs['candidate_binding'] = r['candidate_binding']
    r['evaluation'] = b.evaluate(r)


class RunnerTests(unittest.TestCase):
    def error(self, fn, code=5):
        with self.assertRaises(b.ProtocolError) as cm:
            fn()
        self.assertEqual(cm.exception.code, code)

    @covers('percentile.nearest_rank.p95_n20', 'percentile.median_not_central_mean',
            'percentile.worst_is_max', 'percentile.n19_plus_warmups_insufficient')
    def test_nearest_rank(self):
        c = CASES['percentile.nearest_rank.p95_n20']
        s = b.statistics(c['input']['warm_answer_ns_recorded_order'])
        for k in ('n', 'p95_ns', 'median_ns', 'worst_ns', 'best_ns', 'spread_ns'):
            self.assertEqual(s[k], c['expected'][k])
        self.assertEqual(s['throughput']['denominator'], c['expected']['throughput_elapsed_ns'])
        self.assertNotEqual(s['median_ns'], c['expected']['interpolated_median_forbidden_ns'])
        self.assertNotEqual(s['worst_ns'], s['p95_ns'])
        self.assertEqual(b.statistics([1] * 19)['runner_exit'], 6)
        self.assertIsNone(b.statistics([1] * 19)['p95_ns'])

    @covers('percentile.boundary.p95_equal_budget_passes', 'percentile.over.floor_ms_still_fails')
    def test_p95_boundaries(self):
        for cid in ('percentile.boundary.p95_equal_budget_passes', 'percentile.over.floor_ms_still_fails'):
            c = CASES[cid]
            r = warm_report()
            r['samples'][2:] = [sample('warm_answer', v) for v in c['input']['warm_answer_ns']]
            e = b.evaluate(r)
            self.assertEqual(e['metrics']['warm_answer_p95_ns']['budget_pass'], c['expected']['budget_pass'])
            self.assertEqual(e['runner_exit'], 0)

    @covers('denominator.populations_stay_unmixed')
    def test_disjoint_populations(self):
        c = CASES['denominator.populations_stay_unmixed']
        i, expected = c['input'], c['expected']
        r = report(('warm_answer_p95_ns', 'warm_throughput', 'cold_migration', 'seven_answer_home_ns', 'ui_blocking_ns'))
        for mode, key in (('warmup', 'warmup_ns'), ('warm_answer', 'warm_answer_ns'), ('cold_migration', 'cold_migration_ns'), ('replay', 'replay_ns'), ('seven_answer_home', 'seven_answer_home_ns')):
            r['samples'] += [sample(mode, v) for v in i[key]]
        r['instrumentation']['ui_trace'] = dict(i['ui_trace'], candidate_binding=r['candidate_binding'])
        e = b.evaluate(r)
        self.assertEqual(e['metrics']['warm_answer_p95_ns']['value'], expected['warm_p95_ns'])
        self.assertEqual(e['metrics']['warm_throughput']['value'], {'numerator': 20, 'denominator': 2160000000})
        self.assertEqual(e['metrics']['cold_migration']['value'], {'numerator': 5, 'denominator': 1500})
        self.assertEqual(e['metrics']['seven_answer_home_ns']['value'], expected['seven_answer_home_ns'])
        self.assertEqual(e['metrics']['ui_blocking_ns']['value'], expected['ui_blocking_ns'])
        self.assertTrue(e['measurement_valid'])

    @covers('throughput.exact_20_of_100_passes', 'throughput.79_of_100_fails',
            'throughput.candidate_zero_answers_fails', 'throughput.zero_elapsed_invalid',
            'throughput.zero_baseline_answers', 'throughput.product_not_uint64_wrapped',
            'cold.regression_exact_20_of_100', 'failure.negative_answers')
    def test_regression_integer_math(self):
        for ca, expected in ((80, False), (79, True), (0, True)):
            r = b.regression(ca, 1000, 100, 1000)
            self.assertEqual(r['exceeded'], expected)
            self.assertEqual(r['comparison_status'], 'comparable')
        self.assertEqual(b.regression(80, 1000, 100, 1000)['regression'], {'numerator': 1, 'denominator': 5})
        self.assertIsNone(b.regression(10, 0, 100, 1000)['regression'])
        self.assertIsNone(b.regression(80, 1000, 0, 1000)['regression'])
        self.assertTrue(b.regression(1, 8, 4611686018427387904, 1)['exceeded'])
        self.error(lambda: b.regression(-1, 1000, 100, 1000))
        r = report(('warm_throughput',))
        r['samples'] = [sample('warm_answer', 0)]
        e = b.evaluate(r)
        self.assertEqual(e['status'], 'blocked_unmeasured')
        self.assertIsNone(e['metrics']['warm_throughput']['value'])

    @covers('uint64.max_stored_byte_count_accepted', 'failure.uint64_overflow_not_wrapped', 'failure.uint64_negative')
    def test_uint64_strict(self):
        self.assertEqual(b.uint(18446744073709551615), 18446744073709551615)
        for v in (-1, 18446744073709551616, True, 1.0, '10'):
            self.error(lambda: b.uint(v))

    @covers('clock.monotonic_ignores_wall_jump', 'clock.monotonic_regression_invalid', 'answer.printed_timing_without_monotonic_reading')
    def test_elapsed(self):
        self.assertEqual(b.elapsed(1000, 26000), 25000)
        self.assertIsNone(b.elapsed(5000, 4999))
        self.assertIsNone(b.elapsed(None, None))
        r = report()
        r['samples'] = [sample('warm_answer', None)]
        self.assertEqual(b.evaluate(r)['runner_exit'], 6)

    @covers('answer.parsed_counts', 'answer.corpus_id_mismatch', 'answer.question_id_mismatch',
            'failure.unknown_answer_field', 'failure.answer_status_error_exit_0')
    def test_answer_schema_binding(self):
        c = CASES['answer.parsed_counts']
        parsed = b.parse_answer(b.canonical(c['input']['stdout']))
        self.assertEqual(parsed['record_count'], c['expected']['record_count'])
        a = dict(request(), status='ok', record_count=82, answer_count=1, elapsed_ms=0)
        self.assertNotIn('elapsed_ms', b.parse_answer(b.canonical(a), request()))
        for key, value in (('corpus_id', 'micro-1'), ('question_id', 'q1'), ('status', 'error'), ('extra', 1), ('phase', 'measured')):
            wrong = dict(a, **{key: value})
            self.error(lambda: b.parse_answer(b.canonical(wrong), request()))
        self.error(lambda: b.parse_answer(b'{"protocol":1,"protocol":2}'))
        self.error(lambda: b.parse_answer(b'NaN'))

    @covers('protocol.v070_real_probe', 'failure.old_binary_not_measurable_preport')
    def test_baseline_absent_or_failed(self):
        self.assertEqual(b.baseline_probe(None), {'comparison_status': 'not_measurable_preport', 'baseline_metrics': None})
        failed = b.baseline_probe([sys.executable, str(Path(__file__).with_name('synthetic_candidate.py')), 'fail'])
        self.assertEqual(failed['comparison_status'], 'not_measurable_preport')
        self.assertIsNone(failed['baseline_metrics'])

    @covers('protocol.v080_argv', 'failure.shell_enabled', 'failure.surface_mismatch', 'failure.unlisted_argv', 'failure.manifest_pin_missing')
    def test_surface_manifest(self):
        argv = b.argv_for('candidate', 'req.json')
        self.assertEqual(argv, CASES['protocol.v080_argv']['expected']['argv'])
        b.validate_surface(argv)
        self.error(lambda: b.validate_surface(argv, shell=True))
        self.error(lambda: b.validate_surface(argv, 'v070'))
        self.error(lambda: b.validate_surface(['candidate', 'answer']))
        m = manifest()
        b.validate_manifest(m)
        del m['candidate_source_sha256']
        self.error(lambda: b.validate_manifest(m))
        m = dict(manifest(), extra=1)
        self.error(lambda: b.validate_manifest(m))

    @covers('corpus.micro_conservation', 'failure.impossible_micro_layer')
    def test_golden_micro_exact(self):
        raw = b''.join(b.corpus_bytes('micro'))
        self.assertEqual(raw, (ORACLE / 'golden-micro.jsonl').read_bytes())
        self.assertEqual(len(raw), 12541)
        self.assertEqual(b.digest(raw), 'a56bd9f760544c5426cbebf09d748512d6aaffd84d79b1469a961294f2451d88')
        recs = list(b.records('micro'))
        self.assertEqual(len(recs), 82)
        counts = collections.Counter(r['record_class'] for r in recs)
        self.assertEqual(counts, {'node': 12, 'present_nav': 9, 'missing_nav': 1, 'caller_single': 19, 'caller_multi': 1, 'attempt': 20, 'usage': 20})
        nodes = {r['id'] for r in recs if r['record_class'] == 'node'}
        self.assertTrue(all(r['target'] in nodes for r in recs if 'target' in r and r['target'] is not None))
        self.assertEqual([r['target'] for r in recs if r.get('k') == 18], ['node-micro-8675309-0000-0'])
        self.assertEqual([r['target'] for r in recs if r['record_class'] == 'caller_multi'], ['node-micro-8675309-0000-4'])
        self.error(lambda: b.node_id('micro', 2, 4))

    @covers('corpus.small_census', 'corpus.large_stream_census', 'decode.micro_stream_retains_one_payload')
    def test_full_stream_censuses(self):
        counts = collections.Counter()
        depths = collections.Counter()
        for r in b.records('small'):
            counts[r['record_class']] += 1
            if r['record_class'] == 'node':
                depths[r['depth']] += 1
            if r['record_class'] == 'caller_multi':
                self.assertEqual(len(r['callers']), 2)
            if r['record_class'] == 'attempt':
                self.assertNotIn('quantity', r)
            if r['record_class'] == 'usage':
                self.assertEqual(r['quantity'], 1)
        self.assertEqual(counts, {'node': 11250, 'present_nav': 9000, 'missing_nav': 1000, 'caller_single': 9500, 'caller_multi': 500, 'attempt': 10000, 'usage': 50000})
        self.assertEqual(depths, dict.fromkeys(range(5), 2250))
        counts = collections.Counter()
        first = []
        # Fully enumerate without materializing a million-record list.
        for r in b.records('large'):
            counts[r['record_class']] += 1
            if len(first) < 3:
                first.append(r['id'])
            self.assertNotIn('text', r)
        self.assertEqual(counts, {'fact': 1000000, 'append': 1000})
        self.assertEqual(first, ['fact-large-8675309-0000000', 'fact-large-8675309-0000001', 'fact-large-8675309-0000002'])

    @covers('corpus.seed_identity_profiles_differ', 'corpus.same_identity_same_digest')
    def test_identity(self):
        self.assertNotEqual(b.digest(b.canonical(b.identity('small'))), b.digest(b.canonical(b.identity('large'))))
        self.assertEqual(b.canonical(b.identity('small')), b.canonical(b.identity('small')))
        self.error(lambda: b.identity('small', 1))

    @covers('digest.nist_empty', 'digest.nist_a', 'digest.nist_abc')
    def test_sha256_vectors(self):
        for cid in ('digest.nist_empty', 'digest.nist_a', 'digest.nist_abc'):
            c = CASES[cid]
            self.assertEqual(b.digest(c['input']['utf8'].encode()), c['expected']['sha256'])

    @covers('report.atomic_rename', 'report.no_argv_payload')
    def test_atomic_and_privacy(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / 'report.json'
            dest.write_bytes(b'abc')
            with patch.object(b.os, 'replace', side_effect=OSError('synthetic crash')):
                with self.assertRaises(OSError):
                    b.atomic_json(dest, {'x': 1})
            self.assertEqual(b.file_digest(dest), CASES['report.atomic_rename']['expected']['destination_sha256_after_crash'])
            b.atomic_json(dest, {'x': 1})
            self.assertEqual(dest.read_bytes(), b'{"x":1}')
        r = warm_report()
        r['evaluation'] = b.evaluate(r)
        raw = b.canonical(r)
        for forbidden in (b'req.json', b'raw_argv', b'executable_path', b'credentials'):
            self.assertNotIn(forbidden, raw)
        argv = b.argv_for('candidate', 'req.json')
        self.assertEqual(b.digest(b.canonical(argv)), b.digest(CASES['report.no_argv_payload']['input']['canonical_argv_utf8'].encode()))

    @covers('budgets.seven_equal_passes', 'budgets.seven_plus_one_fails', 'budgets.seven_wrong_answer_count')
    def test_seven_budget(self):
        for ns, passes in ((1000000000, True), (1000000001, False)):
            r = report(('seven_answer_home_ns',))
            r['samples'] = [sample('seven_answer_home', 1), sample('seven_answer_home', ns)]
            e = b.evaluate(r)
            self.assertEqual(e['metrics']['seven_answer_home_ns']['budget_pass'], passes)
            self.assertEqual(e['runner_exit'], 0)
        r['samples'][0]['answer_count'] = 6
        self.error(lambda: b.evaluate(r))

    @covers('budgets.ui_equal_passes', 'budgets.ui_plus_one_fails', 'failure.ui_trace_absent')
    def test_ui_budget_and_binding(self):
        r = report(('ui_blocking_ns',))
        r['samples'] = [sample('ui_blocking', 1)]
        e = b.evaluate(r)
        self.assertEqual(e['runner_exit'], 6)
        self.assertIsNone(e['metrics']['ui_blocking_ns']['value'])
        for ns, passes in ((50000000, True), (50000001, False)):
            r['instrumentation']['ui_trace'] = dict(candidate_binding=r['candidate_binding'], method='ui_event_loop_trace', clock='monotonic', samples_ns=[1, ns])
            e = b.evaluate(r)
            self.assertEqual(e['metrics']['ui_blocking_ns']['value'], ns)
            self.assertEqual(e['metrics']['ui_blocking_ns']['budget_pass'], passes)
        r['instrumentation']['ui_trace']['candidate_binding'] = 'wrong'
        self.error(lambda: b.evaluate(r))

    @covers('budgets.small_resident_equal_passes', 'budgets.small_resident_plus_one_fails',
            'budgets.large_peak_equal_passes', 'budgets.large_peak_plus_one_fails',
            'budgets.small_resident_vmhwm_sampled_passes', 'failure.sampled_labeled_as_peak',
            'failure.resource_unavailable',
            'failure.wrong_memory_scope', 'failure.vmhwm_polling_not_peak')
    def test_memory_methods(self):
        ids = ('budgets.small_resident_equal_passes', 'budgets.small_resident_plus_one_fails', 'budgets.large_peak_equal_passes', 'budgets.large_peak_plus_one_fails', 'budgets.small_resident_vmhwm_sampled_passes')
        for cid in ids:
            c = CASES[cid]
            i = c['input']
            metric = 'small_process_resident_bytes' if 'small' in cid else 'large_reliable_peak_rss_bytes'
            obs = {k: i[k] for k in ('method', 'source', 'scope')}
            obs['value'] = i['bytes']
            for k in ('collector_verified', 'process_handle_held_until_after_exit'):
                if k in i:
                    obs[k] = i[k]
            value = b.memory_metric(metric, obs)
            self.assertEqual(value <= b.CAPS[metric], c['expected']['budget_pass'])
        for source in ('procfs.status.VmRSS', 'procfs.status.VmHWM'):
            obs = dict(value=10, method='reliable_peak', source=source, scope='candidate_process')
            self.error(lambda: b.memory_metric('large_reliable_peak_rss_bytes', obs))
        obs = dict(value=10, method='sampled_maximum', source='procfs.status.VmRSS', scope='candidate_process')
        self.assertIsNone(b.memory_metric('large_reliable_peak_rss_bytes', obs))
        self.assertIsNone(b.memory_metric('small_process_resident_bytes', None))
        obs['scope'] = 'system_commit'
        self.error(lambda: b.memory_metric('small_process_resident_bytes', obs))
        r = report(('large_reliable_peak_rss_bytes',))
        self.assertEqual(b.evaluate(r)['runner_exit'], 6)

    @covers('cold.exactly_five_median', 'failure.cold_four_repetitions', 'failure.cold_six_repetitions')
    def test_cold_population(self):
        s = b.statistics([500, 100, 400, 200, 300], 'cold_migration')
        self.assertEqual((s['n'], s['median_ns'], s['worst_ns']), (5, 300, 500))
        self.assertEqual(b.statistics([100, 200, 300, 400], 'cold_migration')['runner_exit'], 6)
        self.error(lambda: b.statistics([1] * 6, 'cold_migration'))

    @covers('native.linux_does_not_fill_windows', 'native.windows_pass', 'failure.wsl_claimed_as_windows', 'failure.crosscompiled_not_native')
    def test_native_labels(self):
        linux = dict(os_family='linux', environment='native', binary_format='ELF')
        self.assertTrue(b.native_proof(linux, 'linux_native'))
        self.assertFalse(b.native_proof(linux, 'windows_native'))
        self.assertTrue(b.native_proof(dict(os_family='windows', environment='native', binary_format='PE'), 'windows_native'))
        self.assertFalse(b.native_proof(dict(linux, environment='wsl'), 'windows_native'))
        self.assertFalse(b.native_proof(dict(linux, environment='wsl'), 'linux_native'))
        self.assertFalse(b.native_proof(dict(os_family='windows', environment='crosscompiled_foreign', binary_format='ELF'), 'windows_native'))
        actual = b.native_info(sys.executable)
        self.assertEqual(actual['os_family'], 'windows' if os.name == 'nt' else sys.platform)

    @covers('comparison.matching_environment', 'failure.method_mismatch_comparison')
    def test_comparison_context(self):
        i = CASES['comparison.matching_environment']['input']
        self.assertTrue(b.comparable(i['candidate'], i['baseline']))
        for key in b.COMPARABLE:
            changed = dict(i['baseline'], **{key: 'different'})
            self.assertFalse(b.comparable(i['candidate'], changed))
        e = b.evaluate(warm_report())
        self.assertTrue(e['performance_success'])
        self.assertIsNone(b.regression(80, 1000, 100, 1000, equal=False)['regression'])

    @covers('process.harness_plus_candidate_allowed', 'failure.process_limit', 'failure.descendant_collector_absent')
    def test_process_gate(self):
        self.assertTrue(b.process_gate(0, True)['process_budget_pass'])
        self.assertEqual(b.process_gate(1, True)['runner_exit'], 7)
        self.assertIsNone(b.process_gate()['descendant_count'])
        self.assertIsNone(b.process_gate()['process_budget_pass'])
        r = report(('process_tree',))
        e = b.evaluate(r)
        self.assertFalse(e['performance_success'])
        self.assertEqual(e['runner_exit'], 6)
        r['instrumentation']['descendants'] = {'verified': True, 'descendants': 0}
        self.error(lambda: b.evaluate(r))
        for family in ('windows', 'linux'):
            r = report(('process_tree',))
            r['manifest']['os_id'] = family
            r['native'].update(os_family=family, binary_format='PE' if family == 'windows' else 'ELF')
            rebind(r)
            for descendants, total, code in ((0, 2, 0), (1, 3, 7)):
                r['instrumentation']['descendants'] = descendant_evidence(r, descendants)
                e = b.evaluate(r)
                self.assertEqual(e['metrics']['process_tree']['value'], total)
                self.assertEqual(e['metrics']['process_tree']['scope'], 'process_tree')
                self.assertEqual(e['descendant_count'], descendants)
                self.assertEqual(e['process_budget_pass'], descendants == 0)
                self.assertEqual(e['metrics']['process_tree']['budget_pass'], descendants == 0)
                self.assertEqual(e['runner_exit'], code)
            r['samples'] = [sample('warm_answer', 1, termination_reason='timeout', candidate_exit_code=1)]
            self.assertEqual(b.evaluate(r)['runner_exit'], 7)

    def test_process_observation_rejections(self):
        r = report(('process_tree',))
        good = descendant_evidence(r)
        for key, val in (('method', 'process_group'), ('source', 'posix.process_group'),
                         ('scope', 'candidate_process'), ('descendant_count', -1),
                         ('descendant_count', True), ('descendant_count', b.U64),
                         ('collector_verified', 'true'), ('extra', 0),
                         ('method', 'windows_job_object')):
            obs = copy.deepcopy(good)
            obs['observation'][key] = val
            r['instrumentation']['descendants'] = obs
            self.error(lambda: b.evaluate(r))
        r['instrumentation']['descendants'] = dict(good, candidate_binding='unbound')
        self.error(lambda: b.evaluate(r))
        obs = copy.deepcopy(good)
        obs['observation']['collector_verified'] = False
        r['instrumentation']['descendants'] = obs
        e = b.evaluate(r)
        self.assertEqual(e['runner_exit'], 6)
        self.assertIsNone(e['process_budget_pass'])
        self.assertIsNone(e['descendant_count'])

    def test_schema_failure_nulls_all_budgets(self):
        r = synthetic_four_runs()[0]
        r['samples'] += [sample('seven_answer_home', 1, response_schema_valid=False)]
        e = b.evaluate(r)
        self.assertEqual(e['runner_exit'], 5)
        self.assertEqual(e['status'], 'schema_mismatch')
        self.assertFalse(e['measurement_valid'])
        self.assertFalse(e['performance_success'])
        self.assertTrue(all(metric['budget_pass'] is None for metric in e['metrics'].values()))
        self.assertIsNone(e['process_budget_pass'])

    @covers('decode.instruments_independent', 'decode.edge_1002_fails', 'failure.retained_two_fact_payloads')
    def test_decode_counters(self):
        r = report(('retained_context_records', 'max_retained_fact_payload_slots'))
        self.assertEqual(b.evaluate(r)['runner_exit'], 6)
        obs = dict(candidate_binding=r['candidate_binding'], method='independent_decode_counter',
                   source='independent_decode_trace', scope='candidate_process',
                   retained_context_records=1001, max_retained_fact_payload_slots=1,
                   incremental_decode_records=1000, prefix_validation_bytes=64, payload_io_bytes=4096)
        r['instrumentation']['decode'] = obs
        self.assertTrue(b.evaluate(r)['performance_success'])
        self.assertEqual(obs['incremental_decode_records'], 1000)
        self.assertEqual(obs['payload_io_bytes'], 4096)
        obs['retained_context_records'] = 1002
        e = b.evaluate(r)
        self.assertFalse(e['performance_success'])
        self.assertTrue(e['measurement_valid'])
        obs['retained_context_records'] = 1001
        obs['max_retained_fact_payload_slots'] = 2
        self.assertFalse(b.evaluate(r)['performance_success'])

    @covers('failure.large_peak_not_satisfied_by_sample', 'failure.decode_counter_missing')
    def test_missing_metric_author_corrected_oracle_end_to_end(self):
        self.assertIsNone(CASES['failure.large_peak_not_satisfied_by_sample']['expected']['budget_pass'])
        self.assertIsNone(CASES['failure.decode_counter_missing']['expected']['bound_pass'])
        for metric in ('large_reliable_peak_rss_bytes', 'retained_context_records'):
            r = report((metric,))
            if metric == 'large_reliable_peak_rss_bytes':
                r['instrumentation']['large_memory'] = dict(candidate_binding=r['candidate_binding'], observation=dict(
                    value=100, method='sampled_maximum', source='procfs.status.VmRSS', scope='candidate_process'))
            e = b.evaluate(r)
            self.assertEqual(e['status'], 'blocked_unmeasured')
            self.assertEqual(e['runner_exit'], 6)
            self.assertIsNone(e['metrics'][metric]['value'])
            self.assertIsNone(e['metrics'][metric]['budget_pass'])
            self.assertFalse(e['performance_success'])
            self.assertEqual(e['performance_success_false_because'], 'blocked_unmeasured')

    @covers('failure.timeout', 'failure.nonzero_exit', 'failure.signal_or_kill', 'failure.output_limit',
            'failure.precedence_output_limit_before_timeout', 'failure.precedence_timeout_before_nonzero')
    def test_real_subprocess_failures(self):
        helper = str(Path(__file__).with_name('synthetic_candidate.py'))
        for behavior, limit, expected in (('fail', 1000000000, 3), ('timeout', 80000000, 4),
                                           ('flood', 1000000000, 2), ('stderr', 1000000000, 2), ('signal', 1000000000, 3)):
            r = b.execute([sys.executable, helper, behavior], limit, 1024)
            self.assertEqual(r['runner_exit'], expected, behavior)
            self.assertIsNone(r['elapsed_ns'])
            self.assertLessEqual(len(r['stdout']), 1024)
            self.assertLessEqual(r['capture_bytes'], 1025)
        self.assertEqual(b.outcome(output_limit=True, timeout=True, returncode=1)[1], 2)
        self.assertEqual(b.outcome(timeout=True, returncode=1)[1], 4)
        self.assertEqual(b.outcome(returncode=None)[1], 3)
        self.assertEqual(b.outcome(process_limit=True, output_limit=True)[1], 7)
        r = b.execute([sys.executable, helper, 'answer', json.dumps(request())], 2000000000, 65536)
        self.assertEqual(r['runner_exit'], 0)
        self.assertGreater(r['elapsed_ns'], 0)
        self.assertEqual(b.parse_answer(r['stdout'], request())['answer_count'], 1)

    @covers('failure.missing_metric')
    def test_missing_metric_and_tamper(self):
        e = b.evaluate(report())
        self.assertEqual(e['runner_exit'], 6)
        self.assertIsNone(e['metrics']['warm_answer_p95_ns']['value'])
        r = warm_report()
        r['manifest']['candidate_source_sha256'] = 'b' * 64
        self.error(lambda: b.evaluate(r))
        r = warm_report()
        r['samples'][0]['measurement_label'] = 'in_process_warm'
        self.error(lambda: b.evaluate(r))

    def test_cli_import_and_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            r = warm_report()
            r['manifest']['required_metrics'] = ['warm_answer_p95_ns', 'warm_throughput', 'cold_migration']
            r['candidate_binding'] = b.binding_for(r['manifest'])
            r['samples'] += [sample('cold_migration', 1) for _ in range(5)]
            r['evaluation'] = b.evaluate(r)
            b.atomic_json(tmp / 'candidate.json', r)
            baseline = copy.deepcopy(r)
            for s in r['samples']:
                if s['phase'] == 'measured':
                    s['elapsed_ns'] = 2
            r['evaluation'] = b.evaluate(r)
            b.atomic_json(tmp / 'candidate.json', r)
            b.atomic_json(tmp / 'baseline.json', baseline)
            with patch('builtins.print') as output:
                code = b.main(['evaluate', '--report', str(tmp / 'candidate.json'), '--baseline', str(tmp / 'baseline.json')])
            self.assertEqual(code, 0)
            e = json.loads(output.call_args.args[0])
            self.assertFalse(e['performance_success'])
            self.assertTrue(e['comparisons']['warm_throughput']['exceeded'])
            with patch('builtins.print') as output:
                code = b.main(['evaluate', '--report', str(tmp / 'candidate.json'), '--baseline', str(tmp / 'absent.json')])
            self.assertEqual(code, 0)
            self.assertEqual(json.loads(output.call_args.args[0])['comparisons']['comparison_status'], 'not_measurable_preport')
        r = warm_report()
        r['extra'] = 'untrusted'
        self.error(lambda: b.evaluate(r))

    @covers('comparison.absolute_budget_stands_when_refused', 'failure.os_mismatch_comparison')
    def test_refused_os_comparison_preserves_absolute_budget_end_to_end(self):
        candidate, baseline = synthetic_four_runs()[:2]
        # Fixture ordering is frozen: small Windows, then small Linux.
        self.assertNotEqual(candidate['native']['os_family'], baseline['native']['os_family'])
        candidate['samples'] = [dict(s, elapsed_ns=200000000) if s['mode'] == 'warm_answer' else s for s in candidate['samples']]
        candidate['evaluation'] = b.evaluate(candidate)
        comparisons = b.compare_reports(candidate, baseline)
        self.assertEqual(comparisons['warm_throughput']['comparison_status'], 'refused_noncomparable')
        self.assertIsNone(comparisons['warm_throughput']['regression'])
        self.assertTrue(candidate['evaluation']['metrics']['warm_answer_p95_ns']['budget_pass'])
        with tempfile.TemporaryDirectory() as tmp:
            paths = [Path(tmp) / name for name in ('candidate.json', 'baseline.json')]
            for path, r in zip(paths, (candidate, baseline)):
                b.atomic_json(path, r)
            with patch('builtins.print') as output:
                code = b.main(['evaluate', '--report', str(paths[0]), '--baseline', str(paths[1])])
            e = json.loads(output.call_args.args[0])
            self.assertEqual(code, 0)
            self.assertEqual(e['comparison_status'], 'refused_noncomparable')
            self.assertTrue(e['performance_success'])
        r = warm_report()
        r['samples'][0]['phase'] = 'measured'
        self.error(lambda: b.evaluate(r))

    @covers('full_acceptance.four_runs', 'full_acceptance.invalid_run_blocks', 'failure.full_acceptance_missing_windows_large')
    def test_summary_never_bypasses_invalid_or_missing(self):
        self.assertEqual(b.summarize_runs([])['runner_exit'], 6)
        self.assertFalse(b.summarize_runs([])['full_acceptance'])
        reports = synthetic_four_runs()
        self.assertTrue(all(r['evaluation']['performance_success'] for r in reports))
        s = b.summarize_runs(reports)
        self.assertTrue(s['full_acceptance'])
        self.assertTrue(s['measurement_complete'])
        self.assertFalse(s['independent_acceptance'])
        self.assertEqual(s['status'], 'independent_review_required')
        self.assertEqual(s['runner_exit'], 0)
        b.atomic_json(Path(__file__).with_name('synthetic-four-run-fixture.json'), {
            'purpose': 'synthetic computational gate fixture, not native measurement evidence',
            'actual_native_measurements': False, 'reports': reports, 'summary': s})
        s = b.summarize_runs([r for r in reports if not (r['manifest']['profile'] == 'large' and r['native']['os_family'] == 'windows')])
        self.assertEqual(s['missing_runs'], ['large-windows'])
        self.assertIsNone(s['missing_value'])
        self.assertFalse(s['full_acceptance'])
        invalid = copy.deepcopy(reports)
        invalid[1]['instrumentation']['ui_trace'] = None
        invalid[1]['evaluation'] = b.evaluate(invalid[1])
        self.assertEqual(invalid[1]['evaluation']['runner_exit'], 6)
        self.assertIsNone(invalid[1]['evaluation']['metrics']['ui_blocking_ns']['value'])
        self.assertTrue(all(r['evaluation']['performance_success'] for i, r in enumerate(invalid) if i != 1))
        s = b.summarize_runs(invalid)
        self.assertEqual(s['missing_runs'], [])
        self.assertFalse(s['measurement_complete'])
        self.assertFalse(s['full_acceptance'])
        self.assertEqual(s['runner_exit'], 6)

    def test_summary_cross_run_bindings_and_budgets(self):
        for key, value in (('candidate_source_sha256', 'f' * 64), ('corpus_sha256', 'f' * 64)):
            reports = synthetic_four_runs()
            reports[1]['manifest'][key] = value
            rebind(reports[1])
            self.assertTrue(all(r['evaluation']['performance_success'] for r in reports))
            self.error(lambda: b.summarize_runs(reports))
        reports = synthetic_four_runs()
        reports[0]['instrumentation']['small_memory']['observation']['value'] += 1
        reports[0]['evaluation'] = b.evaluate(reports[0])
        self.assertTrue(reports[0]['evaluation']['measurement_valid'])
        self.assertFalse(b.summarize_runs(reports)['full_acceptance'])
        reports = synthetic_four_runs()
        reports[0]['evaluation']['performance_success'] = False
        self.assertFalse(b.summarize_runs(reports)['full_acceptance'])
        self.error(lambda: b.summarize_runs(reports + [reports[0]]))

    def test_run_protocol_with_real_synthetic_transport(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            corpus, req, dest = tmp / 'micro.jsonl', tmp / 'req.json', tmp / 'report.json'
            census = b.generate('micro', corpus)
            m = manifest(('warm_answer_p95_ns', 'warm_throughput', 'cold_migration', 'seven_answer_home_ns', 'process_tree'))
            m.update(candidate_binary_sha256=b.file_digest(sys.executable),
                     runner_sha256=b.file_digest(b.__file__), generator_sha256=b.file_digest(b.__file__),
                     corpus_sha256=census['corpus_sha256'], os_id=b.platform.system().lower(),
                     argv_sha256=b.digest(b.canonical(b.argv_for(sys.executable, req))))
            original = b.execute
            modes = []
            def synthetic_transport(argv, timeout, capture):
                self.assertEqual(argv, b.argv_for(sys.executable, req))
                bound = b.load_json(req)
                modes.append(bound['mode'])
                return original([sys.executable, str(Path(__file__).with_name('synthetic_candidate.py')), 'answer', json.dumps(bound)], timeout, capture)
            with patch.object(b, 'execute', side_effect=synthetic_transport):
                r = b.run_candidate(m, sys.executable, corpus, req, dest)
            self.assertEqual(collections.Counter(modes), {'warmup': 2, 'warm_answer': 20, 'cold_migration': 5, 'replay': 1, 'seven_answer_home': 1})
            self.assertEqual(r['evaluation']['runner_exit'], 6)
            self.assertIsNone(r['evaluation']['metrics']['process_tree']['value'])
            self.assertEqual(b.load_json(dest), r)
            self.assertFalse(r['independent_acceptance'])
            m['argv_sha256'] = '0' * 64
            self.error(lambda: b.run_candidate(m, sys.executable, corpus, req, dest))


# Original author corrected the two fields to null; their amended expectations
# are asserted directly in addition to the existing end-to-end evaluator checks.
UNRESOLVED = {}


if __name__ == '__main__':
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(RunnerTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    exercised = {k: v for k, v in COVERAGE.items() if k not in UNRESOLVED}
    partial = {k: v for k, v in COVERAGE.items() if k in UNRESOLVED}
    unaccounted = sorted(CASES.keys() - exercised.keys() - UNRESOLVED.keys())
    amendments = b.load_json(Path(__file__).with_name('oracle-amendments-pending.json'))
    coverage = {'oracle_sha256': b.file_digest(ORACLE / 'frozen.json'),
                'cases_total': len(CASES), 'covered_semantic_cases': exercised,
                'partially_exercised_unresolved_cases': partial,
                'unresolved_cases': UNRESOLVED, 'unaccounted_cases': unaccounted,
                'tests_run': result.testsRun, 'tests_successful': result.wasSuccessful(),
                'normative_semantic_cases_exercised': len(COVERAGE),
                'pending_exact_field_amendments': amendments['pending_amendments'],
                'resolved_exact_field_amendments': amendments['amendments'],
                'end_to_end_rework_cases': {k: COVERAGE[k] for k in (
                    'process.harness_plus_candidate_allowed', 'failure.process_limit',
                    'failure.descendant_collector_absent', 'full_acceptance.four_runs',
                    'full_acceptance.invalid_run_blocks', 'failure.full_acceptance_missing_windows_large',
                    'comparison.absolute_budget_stands_when_refused', 'failure.os_mismatch_comparison',
                    'failure.large_peak_not_satisfied_by_sample', 'failure.decode_counter_missing')},
                'coverage_is_not_native_measurement_acceptance': True}
    b.atomic_json(Path(__file__).with_name('case-coverage.json'), coverage)
    if unaccounted:
        print('ERROR: unaccounted oracle cases:', unaccounted, file=sys.stderr)
    sys.exit(0 if result.wasSuccessful() and not unaccounted else 1)
