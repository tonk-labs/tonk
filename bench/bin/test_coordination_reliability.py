#!/usr/bin/env python3
"""Deterministic measurement tests; these do not exercise the Tonk product."""
from copy import deepcopy
import json
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import coordination_reliability as reliability


def fixture(registered, evidence_root):
    cases = []
    evidence = []
    for case in registered['cases']:
        evidence_id = case['id'] + '-raw'
        artifact = evidence_root / (case['id'] + '.log')
        artifact.write_text('Synthetic measurement evidence; not native product output.\n')
        evidence.append({'id': evidence_id, 'path': artifact.name,
                         'sha256': hashlib.sha256(artifact.read_bytes()).hexdigest()})
        records = {}
        receipts = []
        for op in case['operations']:
            start = op['scheduled_offset_ms']
            receipts.append({
                'operation_id': op['id'], 'status': 'published_verified',
                'evidence_id': evidence_id,
                'started_ms': start, 'published_ms': start + 20,
                'readback_ms': start + 40, 'finished_ms': start + 50,
                'publication_attempts': 1,
                'local_evaluation_count': 1,
                'recovery': {'required': False, 'resolution': 'not_needed'},
                'readback': {key: op[key] for key in ('artifact_id', 'field', 'payload_sha256')},
            })
            record = records.setdefault(op['artifact_id'], {'artifact_id': op['artifact_id'], 'fields': {}})
            record['fields'][op['field']] = op['payload_sha256']
            if op['shared_value'] is not None:
                record['shared_value'] = op['shared_value']
        cases.append({'id': case['id'], 'receipts': receipts,
                      'evidence_id': evidence_id,
                      'target_identity': {'repository': 'synthetic-' + case['id'], 'build_sha': 'a' * 40},
                      'barriers': [{'round': round_id, 'released_ms': round_id * case['round_interval_ms'],
                                    'ready_ms': {str(writer): round_id * case['round_interval_ms']
                                                 for writer in range(case['writers'])},
                                    'evidence_id': evidence_id} for round_id in range(case['rounds'])],
                      'initial_inventory_complete': True, 'initial_inventory': [],
                      'final_inventory_complete': True, 'final_inventory': list(records.values()),
                      'final_observed_ms': max(r['finished_ms'] for r in receipts) + 1})
        if case['artifact_scope'] == 'case':
            cases[-1]['independent_shared_readbacks'] = [
                {'observer_id': ident, 'replica_id': ident, 'registry_root': str(evidence_root / ident),
                 'repository': cases[-1]['target_identity']['repository'],
                 'synced_ms': 51, 'read_at_ms': 51, 'evidence_id': evidence_id,
                 'shared_value': records[case['id']]['shared_value']}
                for ident in ('replica-a', 'replica-b')]
    return {'plan_sha256': reliability.digest(registered), 'boundary': 'synthetic_fixture',
            'evidence': evidence,
            'adapter_identity': {'command': ['synthetic-measurement-test'], 'source_sha': 'a' * 40,
                                 'source_dirty': False},
            'cases': cases}


class ReliabilityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='reliability-evidence-')
        self.addCleanup(temporary.cleanup)
        self.evidence_root = Path(temporary.name)
        self.plan = reliability.plan(json.loads(reliability.PROTOCOL.read_text()))
        self.observed = fixture(self.plan, self.evidence_root)

    def result(self):
        return reliability.score(self.plan, self.observed, self.evidence_root)

    def test_registered_cases_are_separate_from_quality(self):
        result = self.result()
        self.assertEqual(result['status'], 'passed')
        self.assertEqual([case['planned_operations'] for case in result['cases']], [4, 2, 24])
        self.assertIsNone(result['quality_score'])
        self.assertEqual(result['boundary'], 'synthetic_fixture')
        metric = result['cases'][0]['metrics']
        self.assertEqual(metric['completion_rate'], 1)
        self.assertEqual(metric['latency_ms']['publication']['p95'], 20)
        self.assertEqual(metric['latency_ms']['readback']['p50'], 20)
        self.assertEqual(metric['latency_ms']['end_to_end']['max'], 40)

    def test_lost_field_and_duplicate_records_cannot_pass(self):
        case = self.observed['cases'][0]
        case['final_inventory'][0]['fields'] = {}
        result = self.result()['cases'][0]
        self.assertEqual(result['status'], 'failed')
        self.assertEqual(result['metrics']['lost_independent_fields'], 1)
        case['final_inventory'].append(deepcopy(case['final_inventory'][1]))
        self.assertEqual(self.result()['cases'][0]['metrics']['duplicate_final_records'], 1)

    def test_duplicate_operation_receipt_cannot_pass(self):
        case = self.observed['cases'][0]
        case['receipts'].append(deepcopy(case['receipts'][0]))
        result = self.result()['cases'][0]
        self.assertEqual(result['status'], 'failed')
        self.assertEqual(result['metrics']['duplicate_operation_receipts'], 1)

    def test_failed_write_and_unresolved_recovery_are_counted(self):
        receipt = self.observed['cases'][0]['receipts'][0]
        receipt.update(status='uncertain', recovery={'required': True, 'resolution': 'unresolved'})
        result = self.result()['cases'][0]
        self.assertEqual(result['status'], 'failed')
        self.assertEqual(result['metrics']['recovery_unresolved'], 1)
        self.assertEqual(result['metrics']['completed_operations'], 3)

    def test_resolved_recovery_is_separate_metric(self):
        receipt = self.observed['cases'][0]['receipts'][0]
        receipt.update(publication_attempts=4, recovery={'required': True, 'resolution': 'readback',
                                                       'required_ms': 10, 'resolved_ms': 40})
        result = self.result()['cases'][0]
        self.assertEqual(result['status'], 'passed')
        self.assertEqual(result['metrics']['recovery_required'], 1)
        self.assertEqual(result['metrics']['recovery_resolved'], 1)
        self.assertEqual(result['metrics']['publication_attempts'], 7)

    def test_recovered_readback_does_not_invent_publication_acknowledgment_time(self):
        receipt = self.observed['cases'][0]['receipts'][0]
        receipt.update(published_ms=None, recovery={'required': True, 'resolution': 'readback',
                                                   'required_ms': 25, 'resolved_ms': 40})
        result = self.result()['cases'][0]
        self.assertEqual(result['status'], 'passed')
        metric = result['metrics']
        self.assertEqual(metric['latency_ms']['publication']['count'], 3)
        self.assertEqual(metric['latency_ms']['readback']['count'], 3)
        self.assertEqual(metric['latency_ms']['end_to_end']['count'], 4)
        self.assertEqual(metric['unknown_publication_latency_operations'], [receipt['operation_id']])

    def test_same_artifact_retains_fields_and_accepts_either_shared_winner(self):
        case = self.observed['cases'][1]
        case['final_inventory'][0]['shared_value'] = self.plan['cases'][1]['operations'][0]['shared_value']
        for read in case['independent_shared_readbacks']:
            read['shared_value'] = case['final_inventory'][0]['shared_value']
        self.assertEqual(self.result()['cases'][1]['status'], 'passed')
        case['final_inventory'][0]['shared_value'] = 'invented-value'
        result = self.result()['cases'][1]
        self.assertEqual(result['status'], 'failed')
        self.assertFalse(result['metrics']['shared_field_converged'])

    def test_same_artifact_replay_detected_despite_identical_final_inventory(self):
        self.observed['cases'][1]['receipts'][0]['local_evaluation_count'] = 2
        result = self.result()['cases'][1]
        self.assertEqual(result['status'], 'failed')
        self.assertEqual(result['metrics']['replayed_local_evaluations'], 1)

    def test_missing_evaluation_count_is_unknown_and_cannot_pass(self):
        self.observed['cases'][0]['receipts'][0].pop('local_evaluation_count')
        result = self.result()['cases'][0]
        self.assertEqual(result['status'], 'failed')
        self.assertIsNone(result['metrics']['local_evaluation_count'])
        self.assertEqual(len(result['metrics']['unknown_local_evaluation_counts']), 1)

    def test_pre_dispatch_failure_has_zero_evaluations_without_replay(self):
        receipt = self.observed['cases'][0]['receipts'][0]
        receipt.update(status='failed', local_evaluation_count=0, publication_attempts=0)
        result = self.result()['cases'][0]
        self.assertEqual(result['status'], 'failed')
        self.assertEqual(result['metrics']['local_evaluation_count'], 3)
        self.assertEqual(result['metrics']['replayed_local_evaluations'], 0)
        self.assertEqual(result['errors'], [])

    def test_owner_queue_delay_does_not_claim_native_execution_overlap(self):
        case = self.observed['cases'][1]
        case['receipts'][0].update(native_execution_started_ms=10, native_execution_finished_ms=20)
        case['receipts'][1].update(native_execution_started_ms=20, native_execution_finished_ms=30)
        metric = self.result()['cases'][1]['metrics']
        self.assertEqual(metric['round_overlap_ms'], [50])
        self.assertEqual(metric['native_execution_round_overlap_ms'], [0])
        self.assertEqual(metric['queue_delay_ms']['max'], 20)

    def test_single_inventory_is_not_replica_convergence_proof(self):
        self.observed['cases'][1].pop('independent_shared_readbacks')
        result = self.result()['cases'][1]
        self.assertEqual(result['status'], 'failed')
        self.assertTrue(result['metrics']['chosen_value_valid'])
        self.assertIsNone(result['metrics']['shared_field_converged'])

    def test_duplicate_or_disagreeing_observers_cannot_prove_convergence(self):
        reads = self.observed['cases'][1]['independent_shared_readbacks']
        reads[1]['observer_id'] = reads[0]['observer_id']
        self.assertFalse(self.result()['cases'][1]['metrics']['shared_field_converged'])
        reads[1]['observer_id'] = 'replica-b'
        reads[1]['shared_value'] = 'other-value'
        self.assertFalse(self.result()['cases'][1]['metrics']['shared_field_converged'])

    def test_recovery_resolution_requires_success_and_ordered_timestamps(self):
        receipt = self.observed['cases'][0]['receipts'][0]
        receipt['recovery'] = {'required': True, 'resolution': 'readback', 'required_ms': 40, 'resolved_ms': 10}
        self.assertEqual(self.result()['cases'][0]['status'], 'failed')
        receipt['recovery'].update(required_ms=10, resolved_ms=40)
        receipt['status'] = 'failed'
        self.assertEqual(self.result()['cases'][0]['status'], 'failed')

    def test_dirty_source_requires_patch_hash(self):
        self.observed['adapter_identity']['source_dirty'] = True
        with self.assertRaisesRegex(ValueError, 'patch hash'):
            self.result()
        self.observed['adapter_identity']['source_patch_sha256'] = 'b' * 64
        for case in self.observed['cases']:
            case['target_identity']['source_patch_sha256'] = 'b' * 64
        self.assertEqual(self.result()['status'], 'passed')

    def test_relabeling_same_replica_or_registry_cannot_prove_convergence(self):
        reads = self.observed['cases'][1]['independent_shared_readbacks']
        reads[1]['replica_id'] = reads[0]['replica_id']
        self.assertFalse(self.result()['cases'][1]['metrics']['shared_field_converged'])
        reads[1]['replica_id'] = 'replica-b'
        reads[1]['registry_root'] = reads[0]['registry_root']
        self.assertFalse(self.result()['cases'][1]['metrics']['shared_field_converged'])

    def test_barrier_evidence_required_and_must_follow_readiness(self):
        case = self.observed['cases'][0]
        case['barriers'].pop()
        self.assertIn('missing_or_duplicate_round_barrier', self.result()['cases'][0]['errors'])
        case['barriers'][0]['ready_ms']['0'] = 100
        self.assertIn('invalid_round_barrier:0', self.result()['cases'][0]['errors'])

    def test_raw_evidence_hash_and_operation_reference_required(self):
        self.observed['cases'][0]['receipts'][0]['evidence_id'] = 'missing-raw-log'
        self.assertEqual(self.result()['cases'][0]['status'], 'failed')
        (self.evidence_root / self.observed['evidence'][0]['path']).write_text('Changed after collection')
        with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
            self.result()

    def test_target_build_mismatch_cannot_pass(self):
        self.observed['cases'][0]['target_identity']['build_sha'] = 'c' * 40
        self.assertEqual(self.result()['cases'][0]['status'], 'failed')

    def test_explicit_conflict_is_retained_not_a_successful_independent_write(self):
        self.observed['cases'][1]['receipts'][0]['status'] = 'conflict'
        result = self.result()['cases'][1]
        self.assertEqual(result['status'], 'failed')
        self.assertEqual(result['metrics']['operation_statuses']['conflict'], 1)

    def test_unrun_and_missing_inventory_are_unknown(self):
        self.observed['cases'].pop()
        result = self.result()['cases'][-1]
        self.assertEqual(result['status'], 'unrun')
        self.assertIsNone(result['metrics'])
        self.observed['cases'][0]['final_inventory_complete'] = False
        metric = self.result()['cases'][0]['metrics']
        self.assertIsNone(metric['lost_independent_fields'])
        self.assertIsNone(metric['completion_rate'])

    def test_readback_mismatch_missing_empty_baseline_and_early_inventory_fail(self):
        case = self.observed['cases'][0]
        case['receipts'][0]['readback']['payload_sha256'] = 'b' * 64
        case['initial_inventory_complete'] = False
        case['final_observed_ms'] = 0
        result = self.result()['cases'][0]
        self.assertEqual(result['status'], 'failed')
        self.assertIn('fresh_empty_inventory_not_proven', result['errors'])
        self.assertTrue(any(error.startswith('invalid_readback_evidence') for error in result['errors']))
        self.assertIn('final_inventory_precedes_completion_or_has_no_timestamp', result['errors'])

    def test_serialized_round_cannot_satisfy_concurrency(self):
        receipt = self.observed['cases'][2]['receipts'][1]
        for key in ('started_ms', 'published_ms', 'readback_ms', 'finished_ms'):
            receipt[key] += 50
        self.assertEqual(self.result()['cases'][2]['status'], 'failed')
        self.assertEqual(self.result()['cases'][2]['metrics']['round_overlap_ms'][0], 0)

    def test_slow_serial_rounds_cannot_be_labeled_a_scheduled_burst(self):
        case = self.observed['cases'][2]
        for receipt, op in zip(case['receipts'], self.plan['cases'][2]['operations']):
            for key in ('started_ms', 'published_ms', 'readback_ms', 'finished_ms'):
                receipt[key] += op['round'] * 100
        case['final_observed_ms'] = max(r['finished_ms'] for r in case['receipts']) + 1
        result = self.result()['cases'][2]
        self.assertEqual(result['status'], 'failed')
        self.assertIn('operation_start_outside_registered_schedule', result['errors'])

    def test_rejected_unpublished_field_is_missing_without_inventing_a_lost_commit(self):
        case = self.observed['cases'][0]
        case['receipts'][0]['status'] = 'failed'
        case['final_inventory'].pop(0)
        metric = self.result()['cases'][0]['metrics']
        self.assertEqual(metric['missing_independent_fields'], 1)
        self.assertEqual(metric['lost_independent_fields'], 0)

    def test_mutated_plan_or_unbound_observations_rejected(self):
        self.observed['plan_sha256'] = 'b' * 64
        with self.assertRaisesRegex(ValueError, 'bind'):
            self.result()
        self.observed = fixture(self.plan, self.evidence_root)
        self.plan['cases'][0]['operations'][0]['payload_sha256'] = 'b' * 64
        with self.assertRaisesRegex(ValueError, 'frozen protocol'):
            self.result()

    def test_cli_plan_and_external_receipt_scoring(self):
        with tempfile.TemporaryDirectory(prefix='reliability-cli-') as directory:
            root = Path(directory)
            path = root / 'plan.json'
            script = Path(reliability.__file__)
            created = subprocess.run([sys.executable, str(script), 'plan', '--output', str(path)], capture_output=True)
            self.assertEqual(created.returncode, 0, created.stderr)
            registered = json.loads(path.read_text())
            observations = root / 'observations.json'
            observations.write_text(json.dumps(fixture(registered, root)))
            scored = subprocess.run([sys.executable, str(script), 'score', '--plan', str(path),
                                     '--observations', str(observations)], capture_output=True)
            self.assertEqual(scored.returncode, 0, scored.stderr)
            self.assertEqual(json.loads(scored.stdout)['boundary'], 'synthetic_fixture')
            repeated = subprocess.run([sys.executable, str(script), 'plan', '--output', str(path)], capture_output=True)
            self.assertEqual(repeated.returncode, 2)


if __name__ == '__main__':
    unittest.main()
