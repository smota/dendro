"""Independent draft T14 recipes; stdlib only, never imports/runs product code.

Run: python .agentflow/oracles/v080/T14/fixture-recipes.py
Check without writing: same command with --check
Expected values below are authored constants. Arithmetic checks substantiate the
workings, not product conformance or independent review/acceptance.
"""
from pathlib import Path
from fractions import Fraction as F
import argparse
import hashlib
import json
import uuid
from datetime import datetime, timezone

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
INPUTS = [
    '.agentflow/proposals/next-version-2026-10-22/tasks.json',
    '.agentflow/proposals/next-version-2026-10-22/DESIGN.md',
    '.agentflow/proposals/next-version-2026-10-22/ADVERSARIAL.md',
    'crates/dendro-core/src/domain_v1/quantity.rs',
    'crates/dendro-core/src/domain_v1/revision.rs',
    'crates/dendro-core/src/domain_v1/work.rs',
    'crates/dendro-core/src/domain_v1/record_types/usage.rs',
    'crates/dendro-core/src/domain_v1/record_types/common.rs',
    'crates/dendro-core/src/domain_v1/records.rs',
    'crates/dendro-core/src/domain_v1/mod.rs',
]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def half_even(value, places):
    """Exact quotient/remainder display arithmetic, including negative ties."""
    scaled = value * 10**places
    sign = -1 if scaled < 0 else 1
    q, r = divmod(abs(scaled.numerator), scaled.denominator)
    if 2*r > scaled.denominator or (2*r == scaled.denominator and q % 2):
        q += 1
    return F(sign*q, 10**places)


CASES = []


def case(name, refs, operation, inputs, expected, working, checks=()):
    CASES.append(dict(id='T14-'+name, references=refs, operation=operation,
                      input=inputs, expected=expected, working=working,
                      arithmetic_checks=list(checks)))


case('01-diamond', ['F03', 'T14-A1'], 'direct_and_inclusive',
     dict(direct={'P': 1, 'A': 1, 'B': 1, 'D': 1},
          edges=[['spawn','P','A'], ['spawn','P','B'],
                 ['delegate_existing','A','D'], ['delegate_existing','B','D']],
          D_existing=True, D_spawn_origin_count=0),
     dict(fleet_direct=4, inclusive={'P':4,'A':2,'B':2,'D':1},
          inclusive_nonadditive=True, forbidden_caller_sum=5),
     'Reach(P)={P,A,B,D}; Reach(A)={A,D}; Reach(B)={B,D}. '
     'Each observation occurs once in its set. 1+2+2=5 repeats D and is not fleet usage.',
     ['1+1+1+1 == 4', '1+2+2 == 5'])
case('02-two-roots', ['F03'], 'multi_root_union',
     dict(direct={'R1':2,'R2':3,'D':7},
          edges=[['delegate_existing','R1','D'],['delegate_existing','R2','D']]),
     dict(inclusive={'R1':9,'R2':10}, union_direct=12, forbidden_sum=19),
     'Root observation sets {r1,d} and {r2,d} intersect at d; union is {r1,r2,d}.',
     ['2+7 == 9','3+7 == 10','2+3+7 == 12','9+10 == 19'])
case('03-shared-no-weights', ['F04'], 'allocation',
     dict(shared_tokens=1000, callers=['A','B'], weights=None),
     dict(shared=1000, owned={'A':None,'B':None}, ownership='unknown'),
     'Two callers provide no ownership weights. Shared amount remains 1000, never 500 per caller.')
case('04-partial-weights', ['F04'], 'allocation',
     dict(tokens=1000, weights={'A':'3/5','B':'3/10'}, method='evidenced-v1'),
     dict(allocated={'A':'600/1','B':'300/1'}, residual='100/1', conserved='1000/1'),
     '1000*3/5=600; 1000*3/10=300; residual=1000*(1-9/10)=100.',
     ['1000*F(3,5)==600','1000*F(3,10)==300','1000*(1-F(9,10))==100','600+300+100==1000'])
case('05-omitted-claimant', ['F04'], 'allocation',
     dict(tokens=100, known_callers=['A','B','C'], weights={'A':'25/100','B':'40/100'}),
     dict(allocated={'A':'25/1','B':'40/1'}, residual='35/1', C_owned=None),
     '25/100+40/100=13/20. Residual 100*7/20=35 stays unallocated; C has no weight.',
     ['F(25,100)+F(40,100)==F(13,20)','100*(1-F(13,20))==35','25+40+35==100'])
case('06-overweight', ['F04'], 'allocation',
     dict(tokens=100, weight_sets=[['7/10','4/10'],['70/100','50/100']]),
     dict(allocated=None, refusal='weight_sum_exceeds_one', source_tokens=100),
     '11/10>1 and 6/5>1; neither set admits a conserved nonnegative residual.',
     ['F(7,10)+F(4,10)>1','F(70,100)+F(50,100)>1'])
case('07-fractional-conservation', ['F04'], 'allocation',
     dict(tokens=1, weights={'A':'1/3','B':'1/6'}, display_places=2),
     dict(allocated={'A':'1/3','B':'1/6'}, residual='1/2', total='1/1',
          display={'A':'0.33','B':'0.17','residual':'0.50'}),
     '1/3+1/6=1/2; residual=1/2; exact sum=1. Display does not replace canonical rational shares.',
     ['F(1,3)+F(1,6)+F(1,2)==1','half_even(F(1,3),2)==F(33,100)','half_even(F(1,6),2)==F(17,100)'])
case('08-equivalent-pair', ['F06'], 'observation_equivalence',
     dict(rows=[['o1',100],['o2',100]], equivalence_evidence=[['o1','o2']]),
     dict(canonical_tokens=100, duplicate_rows=1),
     'Explicit equivalence collapses {o1,o2} to one 100-token equivalence class; 2-1=1 duplicate.',
     ['2-1==1'])
case('09-overlap', ['F06'], 'observation_equivalence',
     dict(rows=[['o1',100],['o2',100],['o3',100]], equivalence=[['o1','o2']],
          possible_overlap=[['o1','o3']]),
     dict(canonical_fleet=None, refusal='ambiguous_overlap', equivalent_pair_tokens=100,
          o3_source_tokens=100, duplicate_rows=1),
     'Known equivalence resolves only o1/o2. Unknown intersection with o3 prevents a canonical fleet sum.')
case('10-namespaces', ['F07'], 'request_identity',
     dict(keys=[['authA','inc1','r1',100],['authB','inc1','r1',200]], overlap=False),
     dict(distinct_observations=2, direct_tokens=300),
     'The authority component differs; full keys are unequal. Disjoint direct values add 100+200.',
     ['100+200==300'])
case('11-generation-replacement', ['F06','F09'], 'generation_equivalence',
     dict(rows=[['sourceA',1,'r1',40],['sourceA',2,'r1',40]],
          replacement=True, cross_generation_equivalence=None, possible_same_call=True),
     dict(canonical_fleet=None, refusal='ambiguous_overlap', retained_source_values=[40,40],
          inferred_reset=False),
     'Equal bytes/native keys in generations 1 and 2 prove neither equivalence nor reset; do not silently choose 40 or 80.')
case('12-usage-only', ['T14-A1'], 'direct_usage_without_invocation',
     dict(tokens=17, invocation=None, overlap=False, coverage='complete'),
     dict(direct_tokens=17, linked_invocation_count=0, total_invocation_count=None),
     'Usage amount is evidenced independently. An absent invocation link does not erase 17 or prove total invocation count.')
case('13-inclusion', ['F08'], 'component_total',
     dict(input_inclusive=100, cache_read=40, cache_read_included_in='input', output_exclusive=20),
     dict(derived_total=120, cache_fraction='2/5'),
     'Input already contains cache-read; 100+20=120. Cache fraction=40/100=2/5, not extra usage.',
     ['100+20==120','F(40,100)==F(2,5)'])
case('14-unknown-inclusion', ['F08'], 'component_total',
     dict(input=100, cache_read=40, output=20, inclusion='unknown', literal_reported_total=120),
     dict(derived_total=None, cache_fraction=None, refusal='unknown_inclusion',
          literal_reported_total=120, literal_definition='definition_unknown', retained_components=[100,40,20]),
     'Unknown intersection size prevents union cardinality. Literal 120 is a separate source-grain statement, not a derived total.')
case('15-mixed-knownness', ['F08'], 'component_total',
     dict(modern_input=100, legacy_input=None, legacy_definition='unknown'),
     dict(observed_subset_input=100, full_input=None),
     'Known subset is 100; no quantity is supplied for legacy input, so full total cannot be 100 by substitution.')
case('16-cumulative', ['F09'], 'counter_deltas',
     dict(counter='c1', incarnation='i1', values=[100,150], initialized_since_zero=False),
     dict(first_cumulative=100, first_delta=None, second_delta=50, complete_consumption=None),
     'The first baseline is unknown; only the difference 150-100=50 is observed incremental consumption.',
     ['150-100==50'])
case('17-reset', ['F09'], 'counter_reset',
     dict(previous=150, current=20, source_reset=True, initialized_since_zero=True),
     dict(new_epoch_delta=20),
     'An explicit since-zero reset initializes a new epoch at zero; current-minus-zero=20.', ['20-0==20'])
case('18-decrease-only', ['F09'], 'counter_reset',
     dict(previous=150, current=20, source_reset=False),
     dict(delta=None, status='conflict_or_unknown', retained_current=20),
     '20<150 without reset evidence. Negative difference -130 is not consumed usage and does not prove an epoch change.',
     ['20-150==-130'])
case('19-reset-baseline-only', ['F09'], 'counter_reset',
     dict(previous=150, current=20, source_reset=True, initialized_since_zero=False),
     dict(new_epoch_baseline=20, new_epoch_delta=None),
     'Reset evidence without since-zero initialization does not establish pre-reading consumption in the new epoch.')
case('20-retry', ['F14','T14-A1'], 'same_work_attempt_selection',
     dict(work='W', attempts=[['a1',10,'failed'],['a2',4,'succeeded']],
          edges=[['retry','a1','a2']], accepted_receipt=None),
     dict(work_direct_tokens=14, attempt_count=2, inclusive_a1_tokens=10,
          attempt_successes=1, work_acceptance=None),
     'Work membership selects both distinct attempts: 10+4=14. Retry is not descendant reachability; success is not acceptance.',
     ['10+4==14'])
case('21-resume', ['T14-A1'], 'same_attempt_segments',
     dict(attempt='a1', segments=[['s1',0,3],['s2',5,8]], continuation_evidence=True,
          direct_observations=[['o1',2],['o2',7]]),
     dict(attempt_count=1, segment_count=2, wall_us=6, summed_attempt_us=6, direct_tokens=9),
     'Continuation keeps one attempt. Disjoint lengths (3-0)+(8-5)=6; observations 2+7=9.',
     ['(3-0)+(8-5)==6','2+7==9'])
case('22-correction-cutoffs', ['F22'], 'corrections_at_cutoff_v1',
     dict(event_us=0, revisions=[dict(id='f1',revision=1,ingested=1,value=100),
          dict(id='f2',revision=2,ingested=3,value=80,supersedes='f1'),
          dict(id='f3',revision=3,ingested=4,retracts='f2')], cutoffs=[2,3,4]),
     dict(active_values=[[100],[80],[]], active_fact_counts=[1,1,0], audit_fact_count=3),
     'At cutoff2 only f1 is visible. At3 f2 supersedes f1. At4 f3 removes f2. Empty active set is not erased history.')
case('23-reparent-correction', ['F22','F23'], 'edge_revision_then_closure',
     dict(direct={'P':2,'Q':3,'D':7}, D_created_at=2,
          edge_revisions=[dict(id='e1',revision=1,ingested=1,event=0,spawn=['P','D']),
                          dict(id='e2',revision=2,ingested=3,event=0,spawn=['Q','D'],supersedes='e1')],
          source_key='same-edge-chain', cutoffs=[2,3]),
     dict(at_2={'P':9,'Q':3,'D':7}, at_3={'P':2,'Q':10,'D':7}, fleet_at_both=12),
     'This is an authoritative correction of the same historical relationship, not a new creator. '
     'Cutoff2 has only P->D; cutoff3 only Q->D. Fleet observation set remains {p,q,d}.',
     ['2+7==9','3+7==10','2+3+7==12'])
case('24-conflicting-creators', ['F23'], 'conflicting_spawn_claims',
     dict(direct={'P':2,'Q':3,'D':7},
          edges=[['spawn','P','D'],['spawn','Q','D']], same_D_attempt=True, supersedes=None),
     dict(fleet_direct=12, affected_inclusive=None, refusal='conflicting_creators'),
     'Two independent creator claims violate one-creator cardinality. No latest-wins reparenting; direct facts survive.',
     ['2+3+7==12'])
case('25-cycle', ['F23'], 'cyclic_inclusive',
     dict(edges=[['spawn','A','B'],['spawn','B','A']], direct_after_correction={'A':10,'B':4}),
     dict(fleet_direct=14, inclusive_A=None, inclusive_B=None, refusal='cycle'),
     'A->B->A is not an admissible lineage DAG. Independent direct values remain 10+4=14.', ['10+4==14'])
case('26-return-does-not-expand', ['F23'], 'nonclosure_relationship',
     dict(direct={'A':10,'B':4}, edges=[['spawn','A','B'],['return','B','A']],
          return_correlates_to_spawn=True),
     dict(fleet_direct=14, inclusive={'A':14,'B':4}),
     'Only spawn expands closure. B return A is a receipt, so Reach(B)={B}; no caller-session import.', ['10+4==14'])
case('27-incomplete-closure', ['T14-A1'], 'incomplete_nonnegative_lineage',
     dict(direct_observed=8, known_descendant_observed=3, missing_edge=True,
          all_observed_nonnegative=True, overlap=False),
     dict(inclusive_observed_lower_bound=11, complete_inclusive=None, uncertainty='observed_lower_bound'),
     'The unique observed nonnegative subset contributes 8+3=11; missing descendants cannot reduce it.', ['8+3==11'])
case('28-handoff-straddle', ['T14-A1'], 'assignment_membership',
     dict(worker_tokens=100, evidence_interval_us=[0,10], handoff_us=5,
          ownership_split_evidence=None),
     dict(worker_direct=100, before_owned=None, after_owned=None, shared_unattributed=100),
     'A transfer at5 does not split atomic 100-token evidence into equal halves. Worker-grain observation survives.')
case('29-same-revision', ['F11','F22'], 'revision_identity',
     dict(same_source_key_revision=True, money_values=[['USD',10,100],['USD',1,10]],
          all_other_canonical_fields_equal=True),
     dict(canonical_decimal={'coefficient':1,'denominator':10}, idempotent=True,
          digest_relation='equal', duplicate_consumption=0),
     'Strip one trailing zero from 10/100 ->1/10; numeric canonical content is identical, no new usage.',
     ['F(10,100)==F(1,10)'])
case('30-divergent-revision', ['F22'], 'revision_identity',
     dict(same_source_key_revision=True, values=[40,25], all_other_fields_equal=True),
     dict(status='conflict', selected_value=None, retained_values=[40,25]),
     'Same revision differs in canonical amount. Neither 40,25 nor their sum is an authorized winner.')
case('31-window-moved-correction', ['F22'], 'revision_selection_before_window',
     dict(window_us=[0,5], revisions=[dict(id='a',event=4,value=100,ingested=1),
          dict(id='b',event=6,value=80,ingested=3,supersedes='a')], cutoffs=[2,3]),
     dict(at_2_included_ids=['a'], at_2_tokens=100, at_3_included_ids=[], at_3_active_id='b'),
     'At3 the active revision b lies outside [0,5); filtering b cannot resurrect superseded a.')

case('32-observed-unpriced', ['F11'], 'observed_vs_estimated_cost',
     dict(observed=['USD',2,100], rate_card=None),
     dict(observed={'currency':'USD','coefficient':2,'denominator':100},
          estimate=None, estimate_refusal='rate_card_absent'),
     'Observed two cents needs no price inference. Missing rates invalidate only estimated cost.')
case('33-priced-components', ['F11'], 'estimated_cost',
     dict(provider='p1',model='m1', rate_card='v3', rate_time_match=True,
          exclusive_components={'input':1000,'output':500}, USD_per_million={'input':2,'output':8}),
     dict(estimated_USD={'coefficient':6,'denominator':1000}, exact_reduced='3/500'),
     '1000*2/1000000+500*8/1000000=2000/1000000+4000/1000000=0.006. '
     'Canonical Decimal is 6/1000; reduced ratio 3/500 describes the same number, not Decimal wire.',
     ['1000*F(2,1000000)+500*F(8,1000000)==F(3,500)','F(6,1000)==F(3,500)'])
case('34-cost-partitions', ['F11'], 'cost_partition',
     dict(facts=[['observed','USD',150],['observed','EUR',200],
                 ['estimated','USD',6,'card-v3'],['subscription','USD',2000,'month']], FX_policy=None),
     dict(partitions=[['observed','USD',150],['observed','EUR',200],
                      ['estimated','USD',6],['subscription','USD',2000]], grand_total=None),
     'Currency, evidence mode and subscription period define distinct measures. No operation authorizes 2356 as spend.')
case('35-observed-precedence', ['F11'], 'cost_population_partition',
     dict(observed_USD={'A':'2/100'}, estimated_USD={'A':'3/100','B':'1/100'}),
     dict(preferred_observed_USD='1/50', unmatched_estimated_USD='1/100',
          matched_estimate_USD='3/100', combined_observed_plus_estimated=None),
     'A has an observed comparable cost; its estimate remains labeled and is not added. B has only an unmatched estimate.',
     ['F(2,100)==F(1,50)'])
case('36-unknown-currency', ['F11'], 'cost_currency_eligibility',
     dict(source_amount='0.02', currency=None),
     dict(currency_total=None, refusal='currency_unknown'),
     'A scalar without evidenced currency cannot be silently USD. No expected typed Money or FX value is asserted.')
case('37-missing-component-rate', ['F11'], 'estimated_cost',
     dict(tokens={'input':1000,'output':500}, prices_per_million={'input':2,'output':None}),
     dict(full_estimate=None, refusal='component_rate_missing', priced_input_subset_USD='1/500'),
     'Known input component is 1000*2/1000000=1/500; unknown output price prevents a full estimate.',
     ['1000*F(2,1000000)==F(1,500)'])
case('38-round-once', ['F11'], 'money_display',
     dict(currency='USD', amounts_decimal=[[5,1000],[5,1000]], display_places=2, rounding='half_even'),
     dict(canonical_sum={'coefficient':1,'denominator':100}, display='0.01',
          forbidden_sum_of_rounded='0.00'),
     '0.005+0.005=0.010 ->canonical0.01 ->display0.01. Each 0.005 independently rounds to0.00; '
     'rounding before aggregation loses the cent.',
     ['F(5,1000)+F(5,1000)==F(1,100)','half_even(F(5,1000),2)==0',
      'half_even(F(1,100),2)==F(1,100)'])
case('39-half-even-ties', ['F11'], 'money_display',
     dict(amounts=['0.025','0.035','-0.025','-0.035'], places=2),
     dict(display=['0.02','0.04','-0.02','-0.04']),
     'Scaled ties are2.5,3.5,-2.5,-3.5; choose the even integer2,4,-2,-4. '
     'Signed decimal display is tested without interpreting negative amounts as nonnegative usage.',
     ['half_even(F(25,1000),2)==F(2,100)','half_even(F(35,1000),2)==F(4,100)',
      'half_even(F(-25,1000),2)==F(-2,100)','half_even(F(-35,1000),2)==F(-4,100)'])
case('40-zero-and-precision', ['F11'], 'decimal_normalization',
     dict(decimals=[[0,1000000],[123456789,1000000000],[100,1000]]),
     dict(canonical_decimals=[[0,1],[123456789,1000000000],[1,10]]),
     'Zero is0/1.123456789 has no trailing decimal zero, so preserve all nine source decimal places;100/1000 strips two zeros.',
     ['F(0,1000000)==0','F(100,1000)==F(1,10)'])
case('41-count-overflow', ['T14-A1'], 'checked_count_sum',
     dict(unit='tokens', values=['18446744073709551615','1']),
     dict(canonical_count=None, refusal='overflow', mathematical_sum='18446744073709551616'),
     '2^64-1+1=2^64 exceeds Count u64; never wrap to0.', ['(2**64-1)+1==2**64'])

case('42-full-overlap', ['F01'], 'interval_fold',
     dict(segments=[['A',0,10000000],['B',0,10000000]], clock='c1'),
     dict(wall_us=10000000, summed_attempt_us=20000000, mean_concurrency='2/1'),
     'Union is[0,10M); two distinct attempt lengths sum20M.20M/10M=2.',
     ['10000000+10000000==20000000','F(20000000,10000000)==2'])
case('43-abut', ['F02'], 'interval_fold',
     dict(segments=[['A',0,10000000],['B',10000000,12000000]], clock='c1'),
     dict(wall_us=12000000, summed_attempt_us=12000000, mean_concurrency='1/1', peak=1),
     'At10M A has ended and B starts. Half-open endpoints do not create overlap;10M+2M=12M.',
     ['10000000+2000000==12000000'])
case('44-overlap-sweep', ['F02'], 'interval_fold',
     dict(segments=[['A',0,10000000],['B',9000000,12000000]], clock='c1'),
     dict(wall_us=12000000, summed_attempt_us=13000000, mean_concurrency='13/12', peak=2,
          sweep=[[0,9000000,1],[9000000,10000000,2],[10000000,12000000,1]]),
     'Union[0,12M); lengths10M+3M=13M. Sweep integral9M*1+1M*2+2M*1=13M.',
     ['10000000+3000000==13000000','9000000+2*1000000+2000000==13000000','F(13000000,12000000)==F(13,12)'])
case('45-distinct-vs-replay', ['F01'], 'activity_segment_identity',
     dict(rows=[['A','s1',0,10],['A','s1',0,10],['B','s2',0,10]],
          first_pair_equivalent=True, clock='c1'),
     dict(distinct_segments=2, wall_us=10, summed_attempt_us=20, mean_concurrency='2/1'),
     'Replay of segment s1 is not another execution. A and B remain distinct active attempts even with equal intervals.',
     ['10+10==20'])
case('46-descendant-outlives', ['F05'], 'parent_vs_lineage_wall',
     dict(parent_us=[0,5000000], child_us=[4000000,20000000],
          child_direct_tokens=100, parent_direct_tokens=0, edge='spawn'),
     dict(parent_wall_us=5000000, lineage_wall_us=20000000, lineage_summed_us=21000000,
          inclusive_parent_tokens=100, descendant_outlived=True),
     'Parent length5M; child length16M; connected union[0,20M) length20M; summed=5M+16M=21M.',
     ['5000000+16000000==21000000','20000000-4000000==16000000'])
case('47-duration-window', ['F02','T14-A2'], 'duration_window_intersection',
     dict(window_us=[5,10], segments=[['A',0,8],['B',7,12]], clock='c1'),
     dict(clipped=[[5,8],[7,10]], wall_us=5, summed_attempt_us=6, mean_concurrency='6/5'),
     'Intersect duration intervals geometrically: lengths3+3=6, union[5,10) length5.',
     ['(8-5)+(10-7)==6','10-5==5','F(6,5)==F(6,5)'])
case('48-censored', ['F15'], 'right_censoring',
     dict(start_us=5,end_us=None,cutoff_us=10,liveness_through_us=10,
          liveness_ingested_at=10, knowledge_cutoff=10,clock='c1'),
     dict(observed_contribution_us=5, right_censored=True, status='partial', completed_duration=None),
     'Visible liveness proves survival through10, yielding10-5=5 observed duration; completed duration remains unknown.',
     ['10-5==5'])
case('49-lone-start', ['F15'], 'right_censoring',
     dict(start_us=5,end_us=None,cutoff_us=10,liveness=None),
     dict(duration_us=None,right_censored=False, reason='missing_end'),
     'A start alone proves no ongoing activity through10; do not replace missing end with cutoff.')
case('50-late-liveness', ['F15'], 'sealed_knowledge_cutoff',
     dict(start_us=5,end_us=None,cutoff_us=10,liveness_through_us=10,
          liveness_ingested_at=11, knowledge_cutoff=10),
     dict(duration_us=None,reason='missing_end', used_liveness=False),
     'Receipt11 is outside knowledge cutoff10. Later evidence cannot rewrite sealed earlier knowledge.')
case('51-invalid-and-empty', ['F15'], 'interval_validation',
     dict(intervals_us=[[10,5],[5,5]], clock='c1'),
     dict(reversed={'duration_us':None,'reason':'invalid_interval'},
          empty={'wall_us':0,'summed_attempt_us':0,'mean_concurrency':None,'reason':'zero_denominator'}),
     '10->5 is a conflicting interval, not length0. Valid[5,5) is empty;0/0 has no concurrency ratio.',
     ['5-5==0'])
case('52-unknown-clocks', ['F15'], 'clock_comparability',
     dict(segments=[['A',0,10,'cA'],['B',0,10,'cB']], clock_mapping=None),
     dict(source_durations_us={'A':10,'B':10}, combined_wall_us=None,
          combined_concurrency=None, reason='incomparable_clocks'),
     'Local lengths are10 each. Equal numeric coordinates on unrelated clocks do not establish overlap or a fleet wall.',
     ['10-0==10'])
case('53-clock-bounds', ['F15'], 'boundary_order',
     dict(A={'time_us':10,'error_us':2,'clock':'c'},
          B={'time_us':13,'error_us':2,'clock':'c'}, source_sequence=None),
     dict(A_no_later_than_B=None, status='ambiguous'),
     'A possible[8,12],B possible[11,15] overlap;12<=11 is false and8>15 is false. Nominal ordering is not proved.',
     ['10+2==12','13-2==11','not (12<=11)','not (8>15)'])
case('54-clock-disjoint-and-local', ['F15'], 'boundary_order',
     dict(disjoint={'A':[10,2],'B':[15,2],'same_clock':True},
          local={'A_sequence':4,'B_sequence':5,'same_source_generation':True,'clock':None}),
     dict(disjoint_A_no_later=True, local_A_no_later=True, inferred_causality=False),
     'Latest A=12<=earliest B=13 proves order. Separately sequence4<=5 proves source-local order, not causal influence.',
     ['10+2<=15-2','4<=5'])

for suffix, timestamp, interval, included, reason in [
    ('a',None,[0,10],False,'straddle_unsplit'),
    ('b',4,[0,10],True,None),
    ('c',5,[0,10],False,'timestamp_outside_window'),
    ('d',5,[0,5],False,'timestamp_outside_window'),
    ('e',None,[0,5],True,None),
    ('f',None,None,False,'time_unknown'),
    ('g',0,[0,10],True,None),
    ('h',-1,[0,5],False,'timestamp_outside_window'),
]:
    case('55'+suffix+'-atomic-window', ['T14-A2'], 'atomic_window_selection',
         dict(window_us=[0,5], tokens=100, observed_USD='2/100', event_count=1,
              attribution_timestamp_us=timestamp, evidence_interval_us=interval),
         dict(included_ids=['o1'] if included else [],
              included_amounts={'tokens':100,'observed_USD':'1/50','event_count':1} if included else None,
              excluded_observation_count=0 if included else 1,
              excluded_source_tokens=0 if included else 100, reason=reason,
              coverage='partial' if reason in ('straddle_unsplit','time_unknown') else 'complete_for_declared_selection'),
         'Timestamp takes precedence when present:0<=t<5. Otherwise require full interval containment. '
         'No included observation means exclusion, not an assertion that source usage was measured zero. '
         'Apply the same whole-observation decision to100 tokens,USD0.02 and one event; no proration.')

case('56-ratio-populations', ['F10'], 'named_ratios',
     dict(cost_USD=[10,10,10], success=[1,0,1], closed_attempt_population=['a','b','c']),
     dict(cost_per_success='15/1', mean_successful_attempt_cost='10/1',
          mean_per_attempt_cost_per_success_positive_denominator='10/1', undefined_terms=1),
     'Ratio of sums=(10+10+10)/(1+0+1)=15. Successful-attempt mean=(10+10)/2=10. '
     'Named mean of defined per-attempt ratios=(10/1+10/1)/2=10, excludes one10/0.',
     ['F(10+10+10,1+0+1)==15','F(10+10,2)==10'])
case('57-cache-ratios', ['F10'], 'named_ratios',
     dict(invocations=[['a',90,100],['b',1,10]], inclusion='known_compatible'),
     dict(ratio_of_sums='91/110', named_mean_invocation_fraction='1/2', undefined_terms=0),
     'Sum raw counts:(90+1)/(100+10)=91/110. Mean fractions=(9/10+1/10)/2=1/2. '
     'Reducing individual ratios before summing counts would change the measure.',
     ['F(90+1,100+10)==F(91,110)','(F(90,100)+F(1,10))/2==F(1,2)'])
case('58-mean-missing-grain', ['F10'], 'named_ratios',
     dict(cache_read=91,input_inclusive=110,grain='aggregate',invocation_ids=None,inclusion='known'),
     dict(ratio_of_sums='91/110', named_mean_invocation_fraction=None, refusal='invocation_grain_missing'),
     'Aggregate numerator/denominator suffice for91/110 but cannot reconstruct individual invocation fractions.',
     ['F(91,110)==F(91,110)'])
case('59-undefined-and-zero', ['F10'], 'named_ratios',
     dict(terms=[[9,10],[0,0]], denominator_policy='positive_only'),
     dict(ratio_of_sums='9/10', named_mean_positive_denominator='9/10', included_terms=1, undefined_terms=1,
          all_zero_denominator_ratio=None),
     '0/0 is excluded and counted for the mean. Aggregate9/10 is defined; an all-zero-denominator aggregate is refused.',
     ['F(9+0,10+0)==F(9,10)'])
case('60-missing-denominator', ['F12','F14'], 'missingness',
     dict(successes=2, eligible_attempt_population=None, encountered_accepted=8,
          encountered_rejected=2, expected_source_population=None),
     dict(success_rate=None, completeness_percentage=None, encountered_parse_acceptance='4/5',
          rejected_count=2, denominator_basis='encountered_records'),
     '2 successes lack an eligible outcome denominator.8/(8+2)=4/5 describes encountered parse acceptance only, not sensor completeness.',
     ['F(8,8+2)==F(4,5)'])
case('61-zero-vs-unavailable', ['F12'], 'coverage',
     dict(A={'discovery':'present','read':'complete','projection':'supported','eligible_count':0},
          B={'discovery':'unknown','read':'not_attempted','reason':'log_unavailable'},
          C={'read':'failed'}),
     dict(A={'count':0,'status':'supported'},B={'count':None,'status':'not_observable'},C={'count':None}),
     'Only complete supported eligible enumeration establishes zero. An unread or failed source cannot establish emptiness.')
case('62-population-mismatch', ['F21','T14-A2'], 'ratio_comparability',
     dict(cost={'value':30,'population':['A','B','C'],'unit':'USD','grain':'attempt'},
          denominator={'value':2,'population':['X','Y'],'unit':'accepted_work','grain':'work'},
          attempt_to_work_mapping=None),
     dict(ratio=None,status='noncomparable',reason='unmatched_population_and_grain'),
     '30/2 is arithmetically possible but lacks an evidenced matched work population; no efficiency claim follows.')
case('63-unit-mismatch', ['F21','T14-A2'], 'aggregate_compatibility',
     dict(values=[{'value':100,'unit':'tokens'},{'value':200,'unit':'bytes'}]),
     dict(sum=None, reason='unit_mismatch', retained_values=[100,200]),
     'Tokens and bytes have no evidenced conversion;100+200 cannot be reported as300 tokens.')
case('64-grain-mismatch', ['F21','T14-A2'], 'aggregate_compatibility',
     dict(values=[{'value':2,'unit':'count','grain':'message'},
                  {'value':5,'unit':'count','grain':'fragment'}], fragment_to_message_mapping=None),
     dict(message_count=None,reason='grain_mismatch',retained_values=[2,5]),
     'A fragment is not an additional distinct message; missing assembly/equivalence evidence forbids a7-message total.')
case('65-selection-mismatch', ['F21'], 'descriptive_comparison',
     dict(group_A={'selection':'long_tasks','coverage':'complete'},
          group_B={'selection':'short_tasks','coverage':'partial'}, matched_policy=False),
     dict(status='noncomparable',ranking=None,causal_effect=None,statistical_significance=None),
     'Different selection and coverage prevent the proposed ranking. Raw separately labeled descriptive observations remain possible.')
case('66-zero-acceptance', ['F14','T14-A2'], 'accepted_work_ratio',
     dict(matched_cost_USD=30,known_accepted_work_count=0,closed_world_acceptance=True),
     dict(cost_per_accepted_work=None,reason='zero_denominator',infinity=False),
     '30/0 is undefined; a known zero denominator is distinct from missing acceptance evidence.')
case('67-acceptance-cutoffs', ['F14'], 'outcome_cutoffs',
     dict(attempt='succeeded', transitions=[['completed',1],['accepted',3],['revoked',4]],
          cutoffs=[2,3,4], revocation_targets_acceptance=True),
     dict(attempt_successes=1, acceptance_states=['unknown','accepted','revoked'],merged=None),
     'At2 no acceptance receipt exists. At3 acceptance is visible. At4 its revocation is visible; successful attempt does not change.')
case('68-exact-other-ratios', ['F10'], 'exact_ratio_serialization',
     dict(pairs=[[19,40],[11,16]]),
     dict(reduced=['19/40','11/16'], exact_decimals=['0.475','0.6875']),
     '19/40=475/1000 and11/16=6875/10000 exactly; no binary floating approximation.',
     ['F(19,40)==F(475,1000)','F(11,16)==F(6875,10000)'])

OPEN = [
    dict(id='Q1', topic='General reparenting without authoritative relationship revisions',
         question='Does a runtime reparent event mean a new delegate_existing/handoff relation, or an authoritative correction of historical spawn? Specify stable source key, supersedes target and event validity before expecting subtree transfer.',
         omitted='No generic latest-parent-wins expected vector. Case23 covers only explicit same-chain historical correction; case24 covers conflicting creators.'),
    dict(id='Q2', topic='Money allocation canonical result wire type',
         question='How must a nonterminating rational allocated currency share, e.g. USD0.01 * 1/3 = USD1/300, be serialized? Domain Money.amount is Decimal with power-of-ten denominator; allocation contract requires exact reduced rational shares.',
         omitted='No claimed Money/Decimal wire for1/300. Fractional token allocations and terminating money calculations are covered.'),
    dict(id='Q3', topic='Future T14 adapter and refusal-code ABI',
         question='What exact public accounting_v1 request/response adapter and refusal-code mapping should execute these recipes? No numeric product implementation was used; labels here are semantic draft labels, not promises of Rust symbols.',
         omitted='No claimed native conformance run, frozen fixture digest or builder/reviewer acceptance.'),
]


def build_document():
    return dict(schema_version='dendro.t14.independent-draft-expected/v1', task='T14',
                status='draft_not_frozen_not_reviewed', author_role='independent_expected_value_author',
                builder_role='future_native_grok_builder', revision_policy='corrections_at_cutoff_v1',
                input_sha256={p:sha(ROOT/p) for p in INPUTS},
                conventions={
                    'time':'All JSON times are integer microseconds, including symbolic ingestion/cutoff day labels in F22-derived recipes; no wall-clock date inference.',
                    'identity':'Symbols are synthetic typed identities, not production IDs. Within a recipe repeated symbols identify the same entity. An adapter must materialize domain-tagged IDs through IdentityFactory with a synthetic private test key, never infer from display labels.',
                    'evidence':'Unless explicitly missing/unknown/conflict, positive asserted edges, membership, prices, equivalence and continuation are stipulated source-backed synthetic evidence with valid references. Unspecified metadata is fixture scaffolding, not a default for real data.',
                    'scaffolding':'Give each fact a unique source key/fact/evidence identity, schema/parser version1, revision1, observed_at<=ingested_at, event_sequence in source order. Revision cases override their chain keys and revisions. All ordinary facts are ingested before the query knowledge cutoff; assignments bound exactly their stated work/attempt; all omitted unrelated session facts are out of scope.',
                    'clock':'Interval-fold recipes stipulate one evidenced same clock and exact bounds unless explicitly testing comparability. Arithmetic interval endpoints use UTC microsecond representation, never anonymous cross-clock alignment.',
                    'quantity':'JSON integers are exact; rational strings n/d are reduced mathematical values. Money canonical Decimal uses coefficient/power-of-ten denominator and is distinct from reduced Ratio. Source precision remains metadata.',
                    'assertions':'Expected fields are scoped assertions, not an entire Answer schema. Null never means zero. Refusal/reason strings are semantic draft labels requiring future adapter mapping.',
                    'execution':'fixture-recipes.py materializes input and expected JSON and validates exact arithmetic workings using Python stdlib Fraction. It does not run Dendro or test product behavior. Future builder translates structured operation/input to typed fixtures without editing this author-owned oracle.',
                    'missing_expected':'Open questions are excluded from claimed expectations; do not auto-fill them from implementation output.',
                },
                non_claims=['No causal inference','No statistical significance or confidence',
                            'No implementation correctness claim','No review, freeze, acceptance or publication authority'],
                case_count=len(CASES), cases=CASES, open_questions=OPEN)


def workings_check():
    checks = 0
    # Only author-owned literal expressions, never data read from product output.
    for c in CASES:
        for expression in c['arithmetic_checks']:
            if not eval(expression, {'__builtins__':{}, 'F':F, 'half_even':half_even}):
                raise AssertionError((c['id'], expression))
            checks += 1
    assert len({c['id'] for c in CASES}) == len(CASES)
    return checks


def derivation(document):
    lines = ['# T14 independent expected-value derivation (draft)', '',
             'Authorship is independent of the future Grok builder. No accounting implementation output was read, copied or executed. '
             'Accepted domain_v1 files were inspected as input contracts only; no methods were invoked to compute expectations. '
             'The older broad draft and other oracle values were not read or altered.', '',
             'Run `python .agentflow/oracles/v080/T14/fixture-recipes.py --check` for exact arithmetic and artifact consistency. '
             'Without `--check`, the executable recipes regenerate only these task-owned draft artifacts and a fresh factual authorship receipt. '
             'This is oracle arithmetic verification, not product conformance or independent review.', '',
             '## Contract sources', '',
             '- tasks.json: T14-A1/A2, ownership, stop conditions and identity contract.',
             '- DESIGN.md: section2 canonical quantities; closed WorkEdge contract; corrections and coverage; '
             'section4 accounting/time/cost/outcomes; I02/I03/I04/I10/I12/I18/I20/I21.',
             '- ADVERSARIAL.md: F01-F15 and F21-F24, H3 atomic window selection and H4 decimal normalization. '
             'F13 policy authority and F24 hostile adapter/security specifics are outside numeric T14 ownership; '
             'no kernel/adapter/policy implementation expectations are claimed.',
             '- domain_v1 quantity/revision/work/usage/common declarations: Decimal versus Ratio, units/grain, '
             'assignment closure, typed boundaries and append-only revisions. Input bytes are bound in the JSON/receipt.', '',
             '## Construction and scope', '',
             'See the JSON conventions for executable fixture scaffolding, typed synthetic IDs, stipulated evidence, '
             'clock/window units and adapter obligations. Cases are structured recipes, not raw serde Record JSON. '
             'The future adapter must create complete valid typed envelopes and compare only asserted result fields. '
             'It must not turn missingness or excluded amounts into measured zeros.', '',
             'No generic ambiguous reparenting, nonterminating currency wire value or guessed native ABI is authored. '
             'The explicit questions below park those claims; unambiguous mathematics remains drafted.', '',
             '## Exact workings', '']
    for c in CASES:
        lines += [f"### {c['id']} ({', '.join(c['references'])})", '', c['working'], '',
                  'Expected: `'+json.dumps(c['expected'],ensure_ascii=False,sort_keys=True)+'`', '']
        if c['arithmetic_checks']:
            lines += ['Exact checks: '+'; '.join('`'+s+'`' for s in c['arithmetic_checks']), '']
    lines += ['## Open contract questions (claimed values omitted)', '']
    for q in OPEN:
        lines += [f"- **{q['id']} {q['topic']}**: {q['question']} Omitted: {q['omitted']}"]
    lines += ['', '## Provenance limits', '',
              'No T14 dispatcher launch/session receipt was provided or found in the scoped T14 run paths. '
              'Native client session ID, PID, start time and independently observed model/effort are unavailable. '
              'The model/effort in the receipt are user-requested routing only. The UUID in the receipt is a newly generated '
              'oracle author-run identifier, not a native session or proof of a separate process.', '',
              '`rig whoami --json` failed because rig is not on PATH. '
              '`agentflow-sdlc distribution inspect --target . --json` failed because its installed CLI could not import yaml. '
              'No dependency/environment repair or route migration was attempted. The explicit bounded user assignment '
              'supplies this author role; no OpenRig seat or policy authority is inferred.', '',
              'Only task-owned JSON, derivation, recipe and receipt bytes are written. No product build/tests, '
              'review, freeze, self-acceptance, commit or push is performed. Receipt output hashes exclude the receipt itself '
              'to avoid a self-referential digest; the receipt hash is printed by the recipe command.', '']
    return '\n'.join(lines)


def json_bytes(value):
    return (json.dumps(value,indent=2,ensure_ascii=False)+'\n').encode('utf-8')


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    count=workings_check()
    document=build_document()
    expected_bytes=json_bytes(document)
    derived_bytes=derivation(document).encode('utf-8')
    if args.check:
        assert (HERE/'draft-expected.json').read_bytes()==expected_bytes, 'draft differs from recipes or current input hashes'
        assert (HERE/'derivation.md').read_bytes()==derived_bytes, 'derivation differs from recipes'
        receipt=json.loads((HERE/'author-receipt.json').read_text(encoding='utf-8'))
        assert receipt['input_sha256']==document['input_sha256'], 'input drift'
        for name,digest in receipt['output_sha256'].items():
            assert sha(HERE/name)==digest, ('output drift',name)
    else:
        (HERE/'draft-expected.json').write_bytes(expected_bytes)
        (HERE/'derivation.md').write_bytes(derived_bytes)
        receipt=dict(schema_version='dendro.oracle-author-receipt/v1', task='T14',
                     status='draft_authored_no_review_no_freeze_no_acceptance',
                     unique_worker_run_id='T14-oracle-author-'+str(uuid.uuid4()),
                     author_run_id_kind='locally_generated_uuid_not_native_session_id',
                     receipt_created_at=datetime.now(timezone.utc).isoformat(),
                     role='independent_expected_value_author', provider_family_requested='openai',
                     runtime_requested='codex exec', model_requested='gpt-6.1-sol', effort_requested='medium',
                     runtime_observed='current Codex harness; native dispatcher metadata unavailable',
                     model_observed='unavailable', effort_observed='unavailable',
                     native_client_session_id='unavailable', dispatcher_output='unavailable',
                     dispatcher_input_sha256='unavailable', dispatcher_output_sha256='unavailable',
                     pid='unavailable', process_start_time='unavailable', sanitized_launch_arguments='unavailable',
                     independence=dict(product_output_used=False, product_code_modified=False,
                                       other_oracles_read=False, builder_authorship=False,
                                       reviewer_authorship=False, review_or_freeze_or_acceptance=False,
                                       authenticated_account_separation='not asserted',
                                       execution_disjointness='requires dispatcher/curator verification; not established by role/model text'),
                     environment=dict(rig_identity='unavailable: command not found',
                                      distribution_inspect='failed: ERR_MODULE_NOT_FOUND yaml; no changes made'),
                     owned_path='.agentflow/oracles/v080/T14/**',
                     input_sha256=document['input_sha256'],
                     output_sha256={p:sha(HERE/p) for p in ['draft-expected.json','derivation.md','fixture-recipes.py']},
                     receipt_self_digest='excluded; SHA256 of final receipt emitted by command',
                     verification=dict(exact_arithmetic_checks=count, unique_cases=len(CASES),
                                       product_conformance='not run', independent_review='not performed'),
                     open_question_ids=[q['id'] for q in OPEN])
        (HERE/'author-receipt.json').write_bytes(json_bytes(receipt))
    print(json.dumps(dict(mode='check' if args.check else 'author', cases=len(CASES),
                          arithmetic_checks=count, open_questions=len(OPEN),
                          outputs={p:sha(HERE/p) for p in ['draft-expected.json','derivation.md','author-receipt.json','fixture-recipes.py']})))


if __name__=='__main__':
    main()
