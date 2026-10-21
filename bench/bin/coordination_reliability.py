#!/usr/bin/env python3
"""Plan publication workloads and score externally collected reliability receipts.

This tool never executes writes, invokes a model, or reads credentials.
"""
import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import re


PROTOCOL = Path(__file__).resolve().parents[1] / 'scenarios/coordination-reliability-v1/protocol.json'
STATUSES = {'published_verified', 'failed', 'uncertain', 'conflict'}


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def plan(protocol):
    if not protocol.get('cases'):
        raise ValueError('nonempty reliability cases required')
    cases = []
    for case in protocol['cases']:
        if (type(case['writers']) is not int or case['writers'] < 2
                or type(case['rounds']) is not int or case['rounds'] < 1
                or not number(case['round_interval_ms']) or not number(case['max_start_lateness_ms'])
                or case['artifact_scope'] not in ('case', 'operation')):
            raise ValueError('invalid concurrent workload registration')
        operations = []
        for round_id in range(case['rounds']):
            for writer in range(case['writers']):
                ident = f"{case['id']}/r{round_id:02}/w{writer:02}"
                operations.append({
                    'id': ident, 'writer': writer, 'round': round_id,
                    'scheduled_offset_ms': round_id * case['round_interval_ms'],
                    'artifact_id': case['id'] if case['artifact_scope'] == 'case' else ident,
                    'field': ident, 'payload_sha256': hashlib.sha256(ident.encode()).hexdigest(),
                    'shared_value': ident if case['artifact_scope'] == 'case' else None,
                })
        cases.append(dict(case, operations=operations))
    return {'schema_version': 1, 'protocol': protocol, 'protocol_sha256': digest(protocol), 'cases': cases}


def number(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def distribution(values):
    if not values:
        return {'count': 0, 'p50': None, 'p95': None, 'max': None}
    ordered = sorted(values)
    def percentile(q):
        position = (len(ordered) - 1) * q
        lower = math.floor(position)
        upper = math.ceil(position)
        return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)
    return {'count': len(values), 'p50': percentile(.5), 'p95': percentile(.95), 'max': max(values)}


def score_case(case, observed, evidence_ids, identity):
    expected = {op['id']: op for op in case['operations']}
    if observed is None:
        return {'id': case['id'], 'status': 'unrun', 'planned_operations': len(expected), 'metrics': None}
    errors = []
    target = observed.get('target_identity', {})
    if (not isinstance(target, dict) or not isinstance(target.get('repository'), str) or not target['repository']
            or target.get('build_sha') != identity['source_sha']
            or (identity['source_dirty'] and target.get('source_patch_sha256') != identity['source_patch_sha256'])):
        errors.append('target_build_identity_missing_or_mismatched')
    if not isinstance(target, dict):
        target = {}
    if observed.get('evidence_id') not in evidence_ids:
        errors.append('case_evidence_unbound')
    if observed.get('initial_inventory_complete') is not True or observed.get('initial_inventory') != []:
        errors.append('fresh_empty_inventory_not_proven')
    receipts = observed.get('receipts', [])
    if not isinstance(receipts, list) or any(not isinstance(r, dict) for r in receipts):
        raise ValueError('receipts must be a list')
    ids = Counter(r.get('operation_id') for r in receipts)
    duplicates = sum(count - 1 for count in ids.values() if count > 1)
    if set(ids) - set(expected):
        errors.append('unregistered_operation')
    missing = sorted(set(expected) - set(ids))
    valid = {}
    recovered = required = unresolved = 0
    evaluation_counts = {}
    unknown_evaluations = []
    latency = {'publication': [], 'readback': [], 'end_to_end': []}
    unknown_publication_latency = []
    queue_delays = []
    for receipt in receipts:
        ident = receipt.get('operation_id')
        if ident not in expected or ids[ident] != 1:
            continue
        op = expected[ident]
        if receipt.get('evidence_id') not in evidence_ids:
            errors.append('operation_evidence_unbound:' + ident)
        status = receipt.get('status')
        start, end = receipt.get('started_ms'), receipt.get('finished_ms')
        attempts = receipt.get('publication_attempts')
        evaluations = receipt.get('local_evaluation_count')
        recovery = receipt.get('recovery', {})
        if not isinstance(recovery, dict):
            errors.append('invalid_recovery:' + ident)
            continue
        if status not in STATUSES or not number(start) or not number(end) or end < start:
            errors.append('invalid_receipt:' + ident)
            continue
        if type(attempts) is not int or attempts < 0 or (status == 'published_verified' and attempts < 1):
            errors.append('invalid_attempt_count:' + ident)
            continue
        execution_start = receipt.get('native_execution_started_ms')
        execution_end = receipt.get('native_execution_finished_ms')
        if execution_start is not None:
            if not number(execution_start) or not start <= execution_start <= end:
                errors.append('invalid_native_execution_start:' + ident)
            else:
                queue_delays.append(execution_start - start)
        if execution_end is not None and (not number(execution_start) or not number(execution_end)
                                          or not execution_start <= execution_end <= end):
            errors.append('invalid_native_execution_end:' + ident)
        if evaluations is None:
            unknown_evaluations.append(ident)
        elif type(evaluations) is not int or evaluations < 0:
            errors.append('invalid_evaluation_count:' + ident)
        else:
            evaluation_counts[ident] = evaluations
            if evaluations > 1 or (evaluations == 0 and status != 'failed'):
                errors.append('replayed_or_missing_evaluation:' + ident)
        if recovery.get('required') is False and recovery.get('resolution') == 'not_needed':
            pass
        elif recovery.get('required') is True:
            required += 1
            if recovery.get('resolution') in ('readback', 'retained_publication'):
                recovered += 1
                began, resolved = recovery.get('required_ms'), recovery.get('resolved_ms')
                if (status != 'published_verified' or not number(began) or not number(resolved)
                        or not start <= began <= resolved <= end):
                    errors.append('incoherent_recovery_evidence:' + ident)
            elif recovery.get('resolution') == 'unresolved':
                unresolved += 1
            else:
                errors.append('invalid_recovery:' + ident)
        else:
            errors.append('missing_recovery_evidence:' + ident)
        if status == 'published_verified':
            published, readback = receipt.get('published_ms'), receipt.get('readback_ms')
            expected_readback = {key: op[key] for key in ('artifact_id', 'field', 'payload_sha256')}
            recovered_by_readback = recovery.get('required') is True and recovery.get('resolution') == 'readback'
            if (not number(readback) or not start <= readback <= end
                    or (published is None and not recovered_by_readback)
                    or (published is not None and (not number(published) or not start <= published <= readback))
                    or receipt.get('readback') != expected_readback):
                errors.append('invalid_readback_evidence:' + ident)
                continue
            if published is None:
                unknown_publication_latency.append(ident)
            else:
                latency['publication'].append(published - start)
                latency['readback'].append(readback - published)
            latency['end_to_end'].append(readback - start)
        valid[ident] = receipt
    inventory = observed.get('final_inventory')
    inventory_complete = observed.get('final_inventory_complete') is True and isinstance(inventory, list)
    if inventory_complete and any(not isinstance(row, dict) or not isinstance(row.get('fields'), dict)
                                  for row in inventory):
        raise ValueError('invalid final inventory record')
    final_observed = observed.get('final_observed_ms')
    if (not number(final_observed) or any(final_observed < r['finished_ms'] for r in valid.values())):
        errors.append('final_inventory_precedes_completion_or_has_no_timestamp')
    if not inventory_complete:
        inventory = []
    record_ids = Counter(row.get('artifact_id') for row in inventory)
    duplicate_records = sum(count - 1 for count in record_ids.values() if count > 1)
    records = {row.get('artifact_id'): row for row in inventory if record_ids[row.get('artifact_id')] == 1}
    if set(records) - {op['artifact_id'] for op in expected.values()}:
        errors.append('unregistered_final_artifact')
    lost = 0 if inventory_complete else None
    missing_fields = 0 if inventory_complete else None
    completed = 0
    for ident, op in expected.items():
        receipt = valid.get(ident)
        present = records.get(op['artifact_id'], {}).get('fields', {}).get(op['field']) == op['payload_sha256']
        if inventory_complete and not present:
            missing_fields += 1
            if receipt and receipt['status'] == 'published_verified':
                lost += 1
        if receipt and receipt['status'] == 'published_verified' and present:
            completed += 1
    chosen = shared = None
    if case['artifact_scope'] == 'case' and inventory_complete:
        submitted = {op['shared_value'] for ident, op in expected.items()
                     if ident in valid and valid[ident]['status'] == 'published_verified'}
        chosen_value = records.get(case['id'], {}).get('shared_value')
        chosen = chosen_value in submitted
        reads = observed.get('independent_shared_readbacks', [])
        if not isinstance(reads, list) or any(not isinstance(read, dict) for read in reads):
            raise ValueError('invalid independent shared-field readbacks')
        if reads:
            observer_ids = [read.get('observer_id') for read in reads]
            replica_ids = [read.get('replica_id') for read in reads]
            roots = [read.get('registry_root') for read in reads]
            latest_completion = max((r['finished_ms'] for r in valid.values()), default=0)
            shared = (len(reads) >= 2 and all(isinstance(ident, str) and ident for ident in observer_ids)
                      and len(set(observer_ids)) == len(reads)
                      and all(isinstance(ident, str) and ident for ident in replica_ids)
                      and len(set(replica_ids)) == len(reads)
                      and all(isinstance(root, str) and Path(root).is_absolute() for root in roots)
                      and len({str(Path(root).resolve()) for root in roots}) == len(reads)
                      and all(read.get('shared_value') == chosen_value
                              and number(read.get('read_at_ms'))
                              and number(read.get('synced_ms'))
                              and read['read_at_ms'] >= read['synced_ms'] >= latest_completion
                              and read.get('repository') == target.get('repository')
                              and read.get('evidence_id') in evidence_ids for read in reads))
    overlaps = []
    native_overlaps = []
    for round_id in range(case['rounds']):
        group = [valid.get(op['id']) for op in expected.values() if op['round'] == round_id]
        overlaps.append(None if any(r is None for r in group) else
                        max(0, min(r['finished_ms'] for r in group) - max(r['started_ms'] for r in group)))
        native_overlaps.append(None if any(r is None or not number(r.get('native_execution_started_ms'))
                                           or not number(r.get('native_execution_finished_ms')) for r in group) else
                               max(0, min(r['native_execution_finished_ms'] for r in group)
                                   - max(r['native_execution_started_ms'] for r in group)))
    concurrency = all(value is not None and value > 0 for value in overlaps)
    barriers = observed.get('barriers', [])
    if not isinstance(barriers, list) or any(not isinstance(barrier, dict) for barrier in barriers):
        raise ValueError('invalid barrier evidence')
    barrier_rounds = Counter(barrier.get('round') for barrier in barriers)
    if set(barrier_rounds) != set(range(case['rounds'])) or any(count != 1 for count in barrier_rounds.values()):
        errors.append('missing_or_duplicate_round_barrier')
    for barrier in barriers:
        ready, released = barrier.get('ready_ms'), barrier.get('released_ms')
        group = [valid.get(op['id']) for op in expected.values() if op['round'] == barrier.get('round')]
        if (not isinstance(ready, dict) or set(ready) != {str(i) for i in range(case['writers'])}
                or not number(released) or any(not number(value) or value > released for value in ready.values())
                or barrier.get('evidence_id') not in evidence_ids
                or any(r is None or r['started_ms'] < released for r in group)):
            errors.append('invalid_round_barrier:' + str(barrier.get('round')))
    # Absolute milliseconds share the adapter's monotonic clock. Burst starts
    # must follow the registered schedule rather than becoming a serial loop.
    releases = [barrier['released_ms'] for barrier in barriers if number(barrier.get('released_ms'))]
    origin = min(releases, default=min((r['started_ms'] for r in valid.values()), default=0))
    schedule_delays = {ident: r['started_ms'] - origin - expected[ident]['scheduled_offset_ms']
                       for ident, r in valid.items()}
    if any(not 0 <= delay <= case['max_start_lateness_ms'] for delay in schedule_delays.values()):
        errors.append('operation_start_outside_registered_schedule')
    passed = (not errors and not missing and not duplicates and not duplicate_records
              and inventory_complete and completed == len(expected) and lost == 0
              and unresolved == 0 and not unknown_evaluations and concurrency
              and (case['artifact_scope'] != 'case' or (chosen is True and shared is True)))
    metrics = {'completed_operations': completed if inventory_complete else None,
               'completion_rate': completed / len(expected) if inventory_complete else None,
               'reported_operation_receipts': len(receipts), 'missing_operation_receipts': missing,
               'operation_statuses': dict(Counter(r['status'] for r in valid.values())),
               'publication_attempts': sum(r['publication_attempts'] for r in valid.values()),
               'local_evaluation_count': sum(evaluation_counts.values()) if not unknown_evaluations else None,
               'unknown_local_evaluation_counts': unknown_evaluations,
               'replayed_local_evaluations': sum(max(0, count - 1) for count in evaluation_counts.values()),
               'recovery_required': required, 'recovery_resolved': recovered, 'recovery_unresolved': unresolved,
               'recovery_rate': recovered / required if required else None,
               'duplicate_operation_receipts': duplicates, 'duplicate_final_records': duplicate_records,
               'lost_independent_fields': lost, 'missing_independent_fields': missing_fields,
               'final_inventory_complete': inventory_complete,
               'chosen_value_valid': chosen, 'shared_field_converged': shared, 'round_overlap_ms': overlaps,
               'round_overlap_semantics': 'Admission-to-completion intervals include owner queue time; not native execution overlap.',
               'native_execution_round_overlap_ms': native_overlaps,
               'queue_delay_ms': distribution(queue_delays),
               'unknown_publication_latency_operations': unknown_publication_latency,
               'start_delay_from_schedule_ms': schedule_delays,
               'latency_ms': {key: distribution(values) for key, values in latency.items()}}
    return {'id': case['id'], 'status': 'passed' if passed else 'failed',
            'planned_operations': len(expected), 'errors': errors, 'metrics': metrics}


def bound_evidence(observations, evidence_root):
    entries = observations.get('evidence', [])
    if evidence_root is None or not isinstance(entries, list) or not entries:
        raise ValueError('retained evidence files and evidence root required')
    root = Path(evidence_root).resolve()
    found = set()
    for entry in entries:
        if not isinstance(entry, dict) or not isinstance(entry.get('id'), str) or not entry['id']:
            raise ValueError('invalid evidence entry')
        if entry['id'] in found:
            raise ValueError('duplicate evidence id')
        relative = Path(entry['path'])
        path = root / relative
        if relative.is_absolute() or path.is_symlink() or not path.resolve().is_relative_to(root):
            raise ValueError('evidence path escapes observation root')
        if hashlib.sha256(path.read_bytes()).hexdigest() != entry.get('sha256'):
            raise ValueError('retained evidence SHA-256 mismatch')
        found.add(entry['id'])
    return found


def score(registered, observations, evidence_root=None):
    if not isinstance(registered, dict) or not isinstance(observations, dict):
        raise ValueError('plan and observations must be objects')
    if registered != plan(registered['protocol']):
        raise ValueError('plan does not match its frozen protocol')
    if observations.get('plan_sha256') != digest(registered):
        raise ValueError('observations do not bind this plan')
    boundary = observations.get('boundary')
    if boundary not in ('synthetic_fixture', 'native_integration', 'deployed_service'):
        raise ValueError('explicit exercised boundary required')
    identity = observations.get('adapter_identity')
    if (not isinstance(identity, dict) or not identity.get('command')
            or not isinstance(identity.get('source_sha'), str)
            or not re.fullmatch(r'[a-f0-9]{40}', identity['source_sha'])):
        raise ValueError('adapter command and immutable source identity required')
    if (type(identity.get('source_dirty')) is not bool
            or (identity['source_dirty'] and (not isinstance(identity.get('source_patch_sha256'), str)
                or not re.fullmatch(r'[a-f0-9]{64}', identity['source_patch_sha256'])))):
        raise ValueError('explicit source_dirty and dirty-source patch hash required')
    evidence_ids = bound_evidence(observations, evidence_root)
    cases = observations.get('cases', [])
    if not isinstance(cases, list) or len({c['id'] for c in cases}) != len(cases):
        raise ValueError('duplicate or invalid case observations')
    by_id = {case['id']: case for case in cases}
    if set(by_id) - {case['id'] for case in registered['cases']}:
        raise ValueError('unregistered case observations')
    result = [score_case(case, by_id.get(case['id']), evidence_ids, identity) for case in registered['cases']]
    return {'schema_version': 1, 'plan_sha256': digest(registered), 'boundary': boundary,
            'adapter_identity': identity, 'cases': result,
            'status': 'passed' if all(case['status'] == 'passed' for case in result) else 'incomplete_or_failed',
            'quality_score': None,
            'scope': 'Receipt validation and reliability metrics only. Synthetic fixtures are not product reliability proof.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    create = commands.add_parser('plan')
    create.add_argument('--protocol', type=Path, default=PROTOCOL)
    create.add_argument('--output', type=Path, required=True)
    evaluate = commands.add_parser('score')
    evaluate.add_argument('--plan', type=Path, required=True)
    evaluate.add_argument('--observations', type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == 'plan':
            registered = plan(json.loads(args.protocol.read_text()))
            with args.output.open('x') as stream:
                json.dump(registered, stream, indent=2)
                stream.write('\n')
            print(json.dumps({'plan': str(args.output), 'plan_sha256': digest(registered)}))
            return 0
        result = score(json.loads(args.plan.read_text()), json.loads(args.observations.read_text()), args.observations.parent)
        print(json.dumps(result, indent=2))
        return 0 if result['status'] == 'passed' else 1
    except (ValueError, OSError, KeyError, TypeError) as error:
        parser.exit(2, 'Invalid reliability input: ' + str(error) + '\n')


if __name__ == '__main__':
    raise SystemExit(main())
