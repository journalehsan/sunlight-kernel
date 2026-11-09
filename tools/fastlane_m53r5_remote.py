"""Mapping-specific remote residency, ACK, physical free and reuse joins."""
from fastlane_m53r5_assess import records


def assess_remote(raw, cpus):
    errors = []
    witness = {}
    try:
        rows = [r for r in records(raw, '[R5-REMOTE] ') if r.get('event') != 'controller-complete']
        if [r['event'] for r in rows] != ['resident', 'revoke-committed', 'retired']:
            raise ValueError('missing unique residency/retirement rendezvous')
        resident, committed, retired = rows
        for k in ('generation', 'vm', 'frame'):
            if resident[k] != retired[k] or int(resident[k]) <= 0:
                raise ValueError('retirement differs from resident resource')
        if not 0 <= int(resident['cpu']) < cpus or resident['cpu'] == '1' or resident['targets'] != '2' or cpus < 2:
            raise ValueError('invalid remote placement')
        tlbs = records(raw, '[FL-TLB] ')
        ts = [r for r in tlbs if all(r[k] == resident[k] for k in ('generation','vm','va'))
              and resident['_line'] < r['_line'] < retired['_line']]
        if [r['event'] for r in ts] != ['request', 'ack', 'complete']:
            raise ValueError('missing exact required mapping TLB request/ACK/completion')
        request, ack, done = ts
        if any(r['targets'] != resident['targets'] or r['request'] != request['request'] for r in ts):
            raise ValueError('unrelated invalidation request or targets')
        if ack['cpu'] != '1' or ack['flushed'] != '1' or ack['observed'] != request['request']:
            raise ValueError('target handler did not flush the exact resident mapping')
        if any(r['cpu'] != resident['cpu'] for r in (request, done)):
            raise ValueError('initiator changed across retirement')
        frames = records(raw, '[FL-FRAME] ')
        life = [r for r in frames if r['generation'] == resident['generation'] and r['region'] == '0']
        if [r['event'] for r in life] != ['backing', 'free', 'reuse']:
            raise ValueError('missing same-frame physical release and reuse')
        backing, free, reuse = life
        if any(r['frame'] != resident['frame'] for r in life):
            raise ValueError('physical reuse is not the resident frame')
        if not backing['_line'] < resident['_line'] < committed['_line'] < request['_line'] < ack['_line'] < done['_line'] < free['_line'] < reuse['_line']:
            raise ValueError('ACK/free/reuse ordering not established')
        owners = [r for r in records(raw, '[R51-REUSE] ') if r['channel_generation'] == resident['generation']]
        if [r['event'] for r in owners] != ['claimed', 'released']:
            raise ValueError('missing isolated new owner/release')
        claimed, released = owners
        if any(r['frame'] != resident['frame'] or r['owner_generation'] != claimed['owner_generation'] for r in owners) or int(claimed['owner_generation']) <= 0:
            raise ValueError('new physical owner identity mismatch')
        accesses = [r for r in records(raw, '[FL-ACCESS] ') if r.get('expected') == 'remote-stale' and r['generation'] == resident['generation']]
        faults = [r for r in records(raw, '[FL-PF] ') if all(r.get(k) == resident.get(k) for k in ('pid', 'vm', 'va'))]
        if len(accesses) != 1 or len(faults) != 1 or faults[0].get('inc') != resident['incarnation'] or faults[0].get('error') != '4':
            raise ValueError('missing actual stale read fault on original identity')
        if not reuse['_line'] < claimed['_line'] < retired['_line'] < accesses[0]['_line'] < faults[0]['_line'] < released['_line']:
            raise ValueError('stale access not forced after reuse and before release')
        witness = {'new_owner': claimed, 'new_owner_release': released, 'stale_access': accesses[0], 'hardware_fault': faults[0], 'resident':resident,'tlb_request':request,'remote_handler_ack':ack,
                   'tlb_complete':done,'physical_free':free,'frame_reuse':reuse,
                   'scope':'restricted M2 remote-revocation-fault mapping, real scheduled peer on CPU 1'}
    except (ValueError,KeyError,TypeError) as e:
        errors.append(str(e))
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'witness':witness,
            'historical_failed_attempt':'PRESERVED_FAIL; controlled fixture is separate evidence'}
