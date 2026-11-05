#!/usr/bin/env python3
"""Fail-closed T3R native execution receipts. Synthetic tests are never native proof."""
import re
from threading_t3_validate import assess as assess_topology


def rows(text):
    result = []
    for position, line in enumerate(text.splitlines()):
        if not line.startswith('[T3R] '):
            continue
        line = re.sub(r'vm=AddressSpaceIdentity \{ pml4_phys: (\d+), generation: (\d+) \}',
                      r'root=\1 generation=\2', line)
        parts = line.split()
        event = parts[1]
        fields = {}
        for part in parts[2:]:
            k, v = part.split('=', 1)
            if k in fields:
                raise ValueError('duplicate field')
            fields[k] = v
        result.append((event, fields, position))
    return result


def assess_migration(text, cpus):
    try:
        if cpus not in (2, 4, 12) or not assess_topology(text, ['[T3R] PASS'], cpus):
            return False
        events = rows(text)
        def allof(event):
            return [(d, pos) for e, d, pos in events if e == event]
        def one(event):
            found = allof(event)
            if len(found) != 1:
                raise ValueError(event)
            return found[0]
        def num(d, key):
            value = int(d[key], 0)
            if value < 0:
                raise ValueError('negative')
            return value
        def fixed(d, **expected):
            if any(d.get(k) != str(v) for k, v in expected.items()):
                raise ValueError('inconsistent ' + str(expected))
        begin, begin_pos = one('begin')
        fixed(begin, cpus=cpus)
        owner = num(begin, 'owner'), num(begin, 'inc')
        vm = num(begin, 'root'), num(begin, 'generation')
        if 0 in owner + vm:
            return False
        mapping = {int(a): int(b) for a, b in re.findall(r'^\[CPU-ID\] logical=(\d+) apic=(\d+) bsp=(?:true|false)$',text,re.M)}
        fw = re.findall(r'^\[T3R-FW\] slot=(\d+) processor=(\d+) apic=(\d+) bsp=(\d+)$',text,re.M)
        if len(fw) != cpus or [int(r[0]) for r in fw] != list(range(cpus)):
            return False
        if {int(r[2]) for r in fw} != set(mapping.values()) or any(int(r[3]) != mapping[0] for r in fw):
            return False
        boot = {int(cpu): (int(tss,16),int(df,16),int(nmi,16)) for cpu,_,tss,df,nmi in re.findall(r'^\[CPU-ID\] (?:online|bsp) logical=(\d+) apic=(\d+) tss=(0x[0-9a-f]+) ist_df=(0x[0-9a-f]+) ist_nmi=(0x[0-9a-f]+)$',text,re.M)}
        children = {}
        for d, pos in allof('created'):
            role = num(d,'role')
            if role not in (0,1) or role in children or pos <= begin_pos:
                return False
            fixed(d, owner=owner[0], owner_inc=owner[1], queued=1)
            if (num(d,'root'),num(d,'generation')) != vm:
                return False
            if not all(num(d,k) for k in ('tid','inc','stack','tls')):
                return False
            children[role] = d, pos
        if len(children) != 2 or len({num(d,'tid') for d,_ in children.values()} | {owner[0]}) != 3:
            return False
        if any(len({num(d,k) for d,_ in children.values()}) != 2 for k in ('inc','stack','tls','slot')):
            return False
        def identity(d, role=0):
            child = children[role][0]
            fixed(d,tid=child['tid'],inc=child['inc'])
        def ownership(d, role):
            identity(d,role)
            c = children[role][0]
            fixed(d,stack=c['stack'],tls=c['tls'],rsp0=c['stack'])
            cpu = num(d,'cpu')
            if num(d,'apic') != mapping[cpu] or (num(d,'tss'),num(d,'df'),num(d,'nmi')) != boot[cpu]:
                raise ValueError('topology/ownership')
        dispatches = {}
        for d,pos in allof('dispatch'):
            role,cpu = num(d,'role'),num(d,'cpu')
            ownership(d,role)
            fixed(d,remote_pin='false')
            if (role,cpu) in dispatches or pos <= children[role][1]:
                return False
            dispatches[role,cpu] = pos
        if set(dispatches) != {(0,0),(0,1),(1,0)}:
            return False
        execution = {}
        for d,pos in allof('execution'):
            cpu = num(d,'cpu')
            ownership(d,0)
            if cpu not in (0,1) or cpu in execution or pos <= dispatches[0,cpu]:
                return False
            execution[cpu] = pos
        if set(execution) != {0,1} or mapping[0] == mapping[1]:
            return False
        request,rp = one('request');identity(request)
        fixed(request,source=0,destination=1,blocked='true',pinned='false')
        wake,wp = one('wake');identity(wake)
        fixed(wake,**{'from':0,'queued':1,'request':'true','wait':'cleared','pinned':'false'})
        state,sp = one('state');identity(state)
        fixed(state,**{k:'true' for k in ('tls','fs','gpr','x87','xmm','mxcsr','canary')})
        if not execution[0] < rp < wp < dispatches[0,1] < execution[1] < sp:
            return False
        for event in ('retire','reaped'):
            found = allof(event)
            if len(found) != 2 or {num(d,'role') for d,_ in found} != {0,1}:
                return False
            for d,pos in found:
                role = num(d,'role');identity(d,role)
                if pos <= execution[1] or role == 0 and pos <= sp:
                    return False
                if event == 'retire': fixed(d,count=1)
                else:
                    fixed(d,stack=0,tls=0,current=0,queued=0,pins=0,pending=0,exit=0)
                    retire_pos = next(p for r,p in allof('retire') if num(r,'role') == role)
                    if retire_pos >= pos: return False
        resource, resource_pos = one('resources')
        fixed(resource,workers=0,reservations=0,stacks=1,fd_refs=3,handoff_pins=0,handoff_waiters=0,user_backing='owner_static')
        if num(resource,'heap_final') != num(resource,'heap_base') + num(resource,'cache_bytes'):
            return False
        if num(resource,'frame_base') != num(resource,'frame_final'):
            return False
        pulses = resource['pulses'].split(',')
        if len(pulses) != 3 or any(int(n) <= 0 for n in pulses):
            return False
        end,end_pos = one('PASS');fixed(end,cpus=cpus,executions='0,1',physical_distinct='true')
        if not max(p for _,p in allof('reaped')) < resource_pos < end_pos:
            return False
        allowed = {'begin','created','dispatch','execution','request','wake','state','retire','reaped','resources','PASS'}
        return all(e in allowed for e,_,_ in events)
    except (KeyError, ValueError, TypeError, IndexError, StopIteration):
        return False
