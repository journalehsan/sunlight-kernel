"""Fail-closed auditor for the scoped R2 authenticated hybrid contention fixture."""
import json
from pathlib import Path
import re
import sys


def assess(raw):
    errors = []
    def require(condition, message):
        if not condition:
            errors.append(message)
    def rows(prefix):
        result = []
        for line in raw.splitlines():
            if line.startswith(prefix + ' '):
                pairs = re.findall(r'(\w+)=(\S+)', line)
                row = dict(pairs)
                require(len(pairs) == len(row), 'duplicate receipt field')
                result.append(row)
        return result
    def one(items, **fields):
        found = [r for r in items if all(r.get(k) == str(v) for k, v in fields.items())]
        require(len(found) == 1, 'exactly one ' + str(fields))
        return found[0] if found else {}
    physical = []
    remote = 0
    try:
        races = rows('[FL-R2-RACE]')
        users = rows('[FL-R2]')
        maps = rows('[FL-MAP]')
        tlbs = rows('[FL-TLB]')
        frames = rows('[FL-FRAME]')
        auths = [r for r in rows('[FL-HYBRID-K]') if r.get('event') == 'authorized']
        terminals = [r for r in rows('[FL-HYBRID-K]') if r.get('event') in ('terminal','retired')]
        require(len(auths) == len(terminals) == 14, 'fourteen bound operations and terminals')
        for control in ['invalid-directional-contract', 'cancellation-before-establishment-commit',
                        'foreign-caller', 'foreign-reply-adoption']:
            r = one(users, event='negative', control=control)
            require(r.get('terminal') == 'rejected' and r.get('count') == '1', control)
            if control == 'foreign-caller':
                require(int(r['foreign']) > 0 and r['foreign'] != r['original'], 'distinct foreign caller')
        for mode in range(1, 15):
            armed = one(races, mode=mode, event='armed') if mode not in (7,8) else one(users, mode=mode, event='executed')
            channel = armed['channel']
            a = one(auths, channel=channel)
            if mode not in (7,8):
                require(armed.get('scope') == 'authenticated-ring3/kernel-method-retirement', 'scope boundary')
                for key in ['caller','ci','cv','service','si','sv','policy','pg','endpoint','call_generation']:
                    require(armed[key] == a[key] and int(a[key]) > 0, 'identity join ' + key)
                entered = [one(races, mode=mode, event='entered', actor=actor) for actor in [1,2]]
                require({r['cpu'] for r in entered} == {'0','1'}, 'separate native CPUs')
                winner = 1 if mode % 2 else 2
                one(races, mode=mode, event='lock-held', actor=winner)
                one(races, mode=mode, event='lock-contended', actor=3^winner)
                p = one(races, mode=mode, event='reply-retire-return' if mode>=13 else 'publish-return')
                require((p.get('result') == '0') == (mode>=13 or bool(mode%2)), 'publication/retirement winner')
                require(p.get('duplicate')=='rejected','duplicate native method rejection')
                one(races, mode=mode, event='retire-return')
            require(a['request']=='FastLaneRequired' and a['reply']=='KernelIPC', 'directional contract')
            outcome = 'CallerRetired' if mode==10 else ('ReplyDelivered' if mode in (1,3,5,9,11) else ('ExplicitlyCancelled' if mode==2 else 'PeerClosed'))
            terminal = one(terminals, channel=channel)
            require(terminal['outcome'] == (outcome if mode==10 else f'{outcome}({a["call_generation"]})'), 'exact IPC terminal')
            for key in ['caller','ci','endpoint','call_generation']:
                require(terminal[key] == a[key], 'terminal identity')
            if mode in (9,10):
                reaped=one(users,event='caller-reaped',mode=mode,channel=channel)
                require(reaped['caller']==a['caller'] and reaped['count']=='1','original caller reaped')
                require(not any(r.get('event')=='caller-terminal' and r.get('mode')==str(mode) for r in users),'dead caller cannot accept later reply')
            else:
                accepted = one(users, event='caller-terminal', mode=mode, channel=channel)
                require(accepted['terminal']==outcome and accepted['count']=='1', 'caller terminal')
            if mode<=6 or mode in (9,10,13,14):
                returned = one(users, event='actor-return', mode=mode, channel=channel)
                require(returned['duplicate']=='rejected' and returned['accepted']==str(int(mode in (1,3,5,9))), 'losing/duplicate publication')
            elif mode==8:
                replacement=one(users,event='exec-replacement',channel=channel)
                require(replacement['late']=='rejected' and replacement['reattach']=='rejected','old Actor cannot adopt reply after actual exec')
            stale = one(users, event='negative', control='stale-policy-generation', channel=channel)
            require(stale.get('terminal') == 'rejected' and stale.get('count') == '1', 'stale policy independently rejected')
            inventory = one(rows('[FL-R2-INVENTORY]'), channel=channel)
            for key in ['channels','receipts','waiters','backing','pending_reply','grants','pending','deferred','ordinary','views','policies','policy_refs','deliveries','queued','ipc_waiters','total']:
                require(inventory[key]=='0', 'zero inventory '+key)
            for region in range(3):
                scope = {'generation':channel,'region':region}
                b=one(frames, event='backing', **scope)
                f=one(frames, event='free', **scope)
                require(b['frame']==f['frame'], 'physical lifetime identity')
                for role in range(2):
                    m=one(maps,event='map',role=role,**scope)
                    identity='caller' if role==0 else 'service'
                    require(m['frame']==b['frame'] and m['pid']==a[identity], 'view identity')
                    require(m['inc']==a['ci' if role==0 else 'si'] and m['vm']==a['cv' if role==0 else 'sv'], 'view incarnation')
                    require(m['nx']=='1' and m['write']==str(int((region==1)==(role==1))), 'view rights')
                    view={**scope,'vm':m['vm'],'va':m['va']}
                    request=one(maps,event='request',**view)
                    complete=one(maps,event='complete',**view)
                    tr=one(tlbs,event='request',**view)
                    tc=one(tlbs,event='complete',**view)
                    require(tr['request']==tc['request'] and tr['targets']==tc['targets'], 'TLB lifetime join')
                    mask=int(tr['targets']);remote+=mask.bit_count()
                    acknowledgements=[r for r in tlbs if r.get('event')=='ack' and all(r.get(k)==str(v) for k,v in view.items())]
                    require({int(r['cpu']) for r in acknowledgements}=={c for c in range(64) if mask & (1<<c)}, 'exact remote ACK set')
                    require(len(acknowledgements)==mask.bit_count(), 'no duplicate ACKs')
                    for ack in acknowledgements:
                        require(ack.get('flushed') == '1', 'actual remote invalidation')
                        require(ack['observed']==tr['request'] and int(tr['seq'])<int(ack['seq'])<int(tc['seq']), 'ACK before completion')
                    require(int(b['seq'])<int(m['seq'])<int(request['seq'])<int(tr['seq'])<int(tc['seq'])<int(complete['seq'])<int(f['seq']), 'physical release ordering')
                physical.append({'channel':channel,'region':region,'frame':b['frame'],'release_seq':f['seq']})
        one(users,event='complete',modes=14)
        established=one(rows('[FL-R2-REMOTE]'),event='established')
        scope={'generation':str(2**64-1),'region':0,'vm':established['vm'],'va':established['va']}
        backing=one(frames,event='backing',generation=scope['generation'],region=0)
        mapped=one(maps,event='map',**scope)
        request=one(maps,event='request',**scope)
        target=int(established['targets'])
        require(target>0,'actual remote target required')
        tr=one(tlbs,event='request',**scope)
        tc=one(tlbs,event='complete',**scope)
        acknowledgements=[r for r in tlbs if r.get('event')=='ack' and all(r.get(k)==str(v) for k,v in scope.items())]
        require(int(tr['targets'])==int(tc['targets'])==target,'remote target snapshot')
        require({int(r['cpu']) for r in acknowledgements}=={c for c in range(64) if target&(1<<c)},'remote exact ACK set')
        require(len(acknowledgements)==target.bit_count(),'remote ACK cardinality')
        for ack in acknowledgements:
            require(ack['flushed']=='1' and ack['observed']==tr['request']==tc['request'],'actual target invalidation ACK')
            require(int(tr['seq'])<int(ack['seq'])<int(tc['seq']),'remote ACK ordering')
        done=one(maps,event='complete',**scope)
        release=one(frames,event='free',generation=scope['generation'],region=0)
        require(backing['frame']==mapped['frame']==established['frame'],'remote backing and mapping identity')
        require(int(backing['seq'])<int(mapped['seq'])<int(request['seq'])<int(tr['seq']),'remote mapping before withdrawal')
        require(release['frame']==established['frame'] and int(tc['seq'])<int(done['seq'])<int(release['seq']),'remote physical release ordering')
        remote+=target.bit_count()
        require(not any(x in raw for x in ['PANIC','panicked at','checkpoint=FAIL']), 'guest assertion')
    except (KeyError, ValueError, TypeError) as error:
        errors.append(str(error))
    return {'status':'FAIL' if errors else 'PASS', 'milestone_status':'PARTIAL', 'errors':errors,
            'scope':'fourteen authenticated hybrid operations; native kernel-method lifecycle contenders',
            'physical':physical, 'remote_tlb_targets':remote,
            'remote_ack_proof':'OBSERVED' if remote else 'NOT_EXERCISED',
            'original_controls': {'stale policy generation': 'PASS' if not errors else 'UNACCEPTED',
                                  'invalid directional contract': 'PASS' if not errors else 'UNACCEPTED',
                                  'caller exit before IPC reply': 'PASS' if not errors else 'UNACCEPTED',
                                  'peer closure during transfer': 'PASS' if not errors else 'UNACCEPTED'},
            'caller_exit':'NATIVE_METHOD_CONTENDED', 'service_exec_exit':'NATIVE_EXIT_CONTENDED_EXEC_SEQUENTIAL', 'numeric_pid_reuse':'NOT_RUN'}


if __name__ == '__main__':
    result = assess(Path(sys.argv[1]).read_text(errors='replace'))
    print(json.dumps(result,indent=2))
    raise SystemExit(result['status']!='PASS')
