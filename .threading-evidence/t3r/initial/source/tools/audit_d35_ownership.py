#!/usr/bin/env python3
"""Reproducible source census, not a Rust parser or proof of runtime readiness.

No target/ traversal. Explicitly records unresolved/dynamic calls for review.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent

def read(path):
    return (ROOT / path).read_text()

def location(path, offset):
    source = read(path)
    functions = list(re.finditer(r'\bfn\s+(\w+)\s*\(', source[:offset]))
    return {'file': path, 'line': source.count('\n', 0, offset) + 1,
            'function': functions[-1][1] if functions else 'module/constant'}

def census():
    paths = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard'], cwd=ROOT, text=True).splitlines()
    sources = sorted({p for p in paths if p.endswith('.rs') and (ROOT / p).is_file()})
    calls = []
    for path in sources:
        source = read(path)
        for match in re.finditer(r'\b(nameserver_register|nameserver_lookup(?:_timeout)?|spawn|spawn_from_path(?:_with_\w+)?|spawn_optional|spawn_shell|spawn_tab|handle_spawn_call|exec_into_process|Process::(?:try_)?new)\s*\(', source):
            calls.append(dict(location(path, match.start()), operation=match[1], excerpt=source[match.start():match.start()+220].split(';')[0]))
    manager = 'services/sunlightd/src/main.rs'
    text = read(manager)
    units = []
    # All compiled raw and ordinary multiline definitions, including cfg blocks.
    pattern = r'(?:let\s+\w+\s*=\s*)(?:r#"(.*?)"#|"((?:[^"\\]|\\.)*)")\s*;'
    for m in re.finditer(pattern, text, re.S):
        definition = m[1] if m[1] is not None else bytes(m[2], 'utf8').decode('unicode_escape')
        if '[Service]' not in definition or 'ExecStart=' not in definition:
            continue
        fields = {}
        for line in definition.splitlines():
            if '=' in line:
                key, value = line.split('=', 1)
                fields.setdefault(key, []).extend(value.split())
        end = text.find('\n    }', m.end())
        tail = text[m.end():end+6]
        units.append({'executable': fields['ExecStart'][0], 'fields': fields,
                      'default_enabled': 'entry.enabled = false' not in tail,
                      'definition': location(manager, m.start()),
                      'conditional': 'document-service' if 'sunlight-docserv' in definition else 'document-ui-gate' if 'sunlight-docview' in definition else None})
    init_path = 'services/init/src/main.rs'
    init_source = read(init_path)
    init_block = re.search(r'const INIT_SERVICES.*?=\s*\[(.*?)\];', init_source, re.S)
    init_paths = re.findall(r'"(/[^"\n]+)"', init_block[1])
    # Cargo target catalog includes services not embedded in the kernel.
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=ROOT, text=True))
    binaries = []
    for package in metadata['packages']:
        for target in package['targets']:
            if 'bin' in target['kind']:
                binaries.append({'name': target['name'],
                    'source': str(Path(target['src_path']).relative_to(ROOT)),
                    'manifest': str(Path(package['manifest_path']).relative_to(ROOT)),
                    'required_features': target.get('required-features', [])})
    embedded = []
    kernel_text = read('kernel/src/main.rs')
    for m in re.finditer(r'static\s+(\w+).*?include_bytes!\("([^"]+)"\)', kernel_text, re.S):
        embedded.append({'symbol': m[1], 'artifact': m[2], 'evidence': location('kernel/src/main.rs', m.start()), 'creates_process': False})
    return sources, calls, units, init_paths, binaries, embedded

# Current privilege and bootstrap assessment. A missing complete lifecycle
# contract is a blocker, not an invitation to guess a capability profile.
ASSESS = {
 'init': ('BootstrapRequired', True, True, 'Kernel spawn token; nameserver validation authority', 'Kernel scheduler/IPC bootstrap', 'Nameserver IPC receive loop; no parent supervisor', 'Root nameserver and trusted SunlightD handoff; retain'),
 'vfs_server': ('BootstrapRequired', True, True, 'Read-only FAT boot-share mapping; VFS operations', 'Kernel FAT/block setup, Init registration', 'Registers vfs', 'SunlightD resolves User=root through VFS; boot-share mapping has no equivalent delegated launch contract'),
 'tty_server': ('BootstrapRequired', False, True, 'Kernel-mapped framebuffer, initial geometry arguments, trusted_tty_session_service marker', 'Init, VFS, input drivers; UAC/display/session for login', 'Registers tty, then login loop', 'Retain login/diagnostic path; authentication and graphical handoff need separate proof'),
 'timer_server': ('KeepExistingWithReason', False, False, 'Timer IPC and kernel name-routed tick delivery', 'Init registration; kernel timer ticks', 'Registers time', 'Early by existing policy; SunlightD deadlines use kernel syscalls, not this server. No unit or complete restart contract; preserve phase gate/tick semantics pending separate proof'),
 'sunlightd': ('BootstrapRequired', False, False, 'Init trust-chain manager marker, spawn, VFS root credentials, lifecycle validation', 'Init nameserver/spawn, VFS; kernel timer syscalls for deadlines', 'Registers before loading units; sunlightd OK precedes spawn-cap wait', 'Exactly one Init list entry; no recovery/adoption or validated ready handoff; must implement before further cleanup'),
 'sunlight-kbd': ('KeepExistingWithReason', False, True, 'IRQ subscription / keyboard input syscalls', 'tty; optional deviced reporting', 'Waits for tty; no public service registration', 'Early login input path; no unit or restart/resource cleanup contract'),
 'sunlight-mouse': ('KeepExistingWithReason', False, True, 'Name-gated PS/2 port/init access', 'tty; display and deviced/niced optional discovery', 'Registers mouse_driver', 'Input teardown/reinitialization and privilege profile unproven'),
 'sunlight-usb-mouse': ('KeepExistingWithReason', False, True, 'Name-gated xHCI MMIO/DMA device grant', 'tty; optional deviced reporting', 'No public nameserver registration', 'DMA/controller reset and safe restart cleanup unproven'),
 'sunlight-swapd': ('KeepExistingWithReason', False, True, 'Trusted embedded SwapAdmin path', 'Kernel RAM/CPU/zram diagnostics', 'Kernel policy health check then exit(0)', 'OneShot immutable boot policy; moving later changes reclaim availability; no retry policy established'),
 'deviced': ('KeepExistingWithReason', False, True, 'Name-gated hardware inventory snapshot', 'Kernel inventory; driver reports', 'Registers deviced', 'Driver incarnation cleanup and registration/restart contract not proven'),
 'net_server': ('KeepExistingWithReason', False, True, 'Name-gated kernel network frame proxy', 'Kernel NIC; Init registration; networkd/resolved integrate asynchronously', 'Registers net', 'Sockets/DHCP/packet state and restart recovery need independent native gate'),
 'networkd': ('KeepExistingWithReason', False, False, 'Network control, device inventory and resolver access; Init currently unrestricted profile', 'deviced, net, resolved (bounded reconciliation)', 'Registers networkd', 'No manifest; secure lookup profile and reconciliation after restart unproven'),
 'resolved': ('KeepExistingWithReason', False, False, 'Resolver configuration / kernel DNS interfaces; unrestricted Init profile', 'Network configuration; kernel DNS', 'Registers resolved', 'No manifest; DNS readiness differs from endpoint publication; restart test required'),
 'powerd': ('KeepExistingWithReason', False, True, 'Power policy kernel interfaces; unrestricted Init profile', 'Kernel power/CPU controls', 'Registers powerd', 'No manifest or generation-aware recovery contract; assess separately'),
 'thermald': ('KeepExistingWithReason', False, True, 'Thermal sensors/fan control interfaces; unrestricted Init profile', 'powerd, kernel thermal backend', 'Registers thermald', 'Safety policy reapplication and failure recovery not proven; keep resident Init path'),
 'audiod': ('KeepExistingWithReason', False, True, 'Name-gated HDA/ES1371 device MMIO/DMA grant', 'Kernel hardware discovery, client shared buffers', 'Registers audiod and audiod.capture', 'DMA stop, capture sessions and peer buffer cleanup need dedicated restart gate'),
 'sunlight-display': ('KeepExistingWithReason', False, True, 'Trusted display path, GPU proxy/MMIO/framebuffer', 'tty/login, input, mezzo/session clients', 'Registers display_server', 'Compositor surfaces, display authority and session teardown need dedicated validation'),
 'mezzo': ('KeepExistingWithReason', False, False, 'Session lock mediation, display authority, manager lock-presenter control', 'display_server, sunlightd, UAC authenticated grants', 'Registers mezzo', 'Lock authority cleanup and capability profile incomplete; no mass migration'),
 'pty_server': ('KeepExistingWithReason', False, True, 'Trusted PTY path and generation probes', 'Kernel PTY subsystem, Init', 'Registers pty', 'PTY ownership/close/hangup and terminal regression required before migration'),
}

def build():
    sources, calls, units, init_paths, binaries, embedded = census()
    services = []
    names = ['init', 'vfs_server', 'tty_server'] + [Path(p).name for p in init_paths] + [Path(u['executable']).name for u in units]
    aliases = {'sunlight-sessiond': ['sunlight.session.v1'], 'sunlight-docserv': ['docserv'], 'wiseowl-braind': ['wiseowl.brain.v1', 'wiseowl.display-lifecycle.v1'], 'wiseowl-memorydb': ['wiseowl.memorydb.v1', 'wiseowl.memorydb.authority.v1', 'wiseowl-memorydb'], 'wiseowl-indexd': ['wiseowl.index.v1', 'wiseowl-indexd']}
    for name in dict.fromkeys(names):
        unit = next((u for u in units if Path(u['executable']).name == name), None)
        binary = next((b for b in binaries if b['name'] == name), None)
        if not binary:
            mapped = {'init':'sunlight-init','vfs_server':'sunlight-vfs-server','tty_server':'sunlight-tty-server','timer_server':'sunlight-timer-server'}
            binary = next((b for b in binaries if b['name'] == mapped.get(name)), None)
        source = binary['source'] if binary else ('services/sunlight-vortex-shell/src/main.rs' if name=='vortex-lock-presenter' else None)
        related = [c for c in calls if source and (c['file'] == source or (name.startswith('wiseowl-') and c['file'].startswith(str(Path(source).parents[1]))))]
        registrations = [c for c in related if c['operation'] == 'nameserver_register']
        lookups = [c for c in related if c['operation'].startswith('nameserver_lookup')]
        fields = unit['fields'] if unit else {}
        launcher = 'Kernel' if name in ('init','vfs_server','tty_server') else 'Init' if '/sbin/'+name in init_paths else 'SunlightD'
        if unit:
            classification, before, privileged = 'SunlightDResident', False, False
            capability = fields.get('Capability', [])
            deps = {'requires': fields.get('Requires', []), 'after': fields.get('After', []), 'wants_advisory': fields.get('Wants', [])}
            ready = 'Type='+fields.get('Type',['simple'])[0]+'; simple is Running on spawn; dependencies gate on nameserver aliases (not D1 verified readiness)'
            reason = 'Already managed; preserve resident policy and existing privileges. No D2 drain contract proven.'
            risk = 'Retain; manager recovery and generation-aware resident supervision remain blockers'
            if not unit['default_enabled']:
                reason += ' Disabled by default; administrative Start only.'
        else:
            classification, before, privileged, capability, deps, ready, reason = ASSESS[name]
            risk = 'Retain bootstrap' if classification == 'BootstrapRequired' else 'High or undemonstrated; no migration'
        if launcher == 'Kernel':
            launch = {'file':'kernel/src/main.rs','function':'_start (boot entry)', 'search': 'Process::new('+ {'init':'1','vfs_server':'3','tty_server':'4'}[name]}
        elif launcher == 'Init':
            launch = {'file':'services/init/src/main.rs','function':'_start -> spawn_service','list_index':init_paths.index('/sbin/'+name)}
        else:
            launch = {'file':'services/sunlightd/src/main.rs','function':'autostart_services / handle_control_message -> spawn_service_at -> spawn_named_with_identity','definition':unit['definition']}
        conflicts = [c for c in calls if c['operation']=='spawn' and re.search(r'/s?bin/'+re.escape(name)+r'"', c['excerpt'])]
        conditional_probes = [c for c in conflicts if c['file'] in ('sunlight-docview/src/main.rs', 'sunlight-docserv/src/main.rs')]
        conflicts = [c for c in conflicts if c not in conditional_probes]
        services.append({'name':name,'executable': unit['executable'] if unit else ('embedded:sunlight-'+name if name in ('init','vfs_server','tty_server') else '/sbin/'+name), 'source':source,'launch_site':launch,'launcher':launcher,
            'supervisor':'SunlightD (resident legacy lifecycle)' if launcher=='SunlightD' else None,
            'registered_names':aliases.get(name, re.findall(r'nameserver_register\(\s*"([^"]+)"', '\n'.join(c['excerpt'] for c in registrations))),
            'endpoint':'Runtime-created owner endpoint; numeric ID is per incarnation' if registrations else 'See registration evidence; no fixed endpoint assumed',
            'registration_evidence':registrations,'capability_requirements':capability,'dependencies':deps,'runtime_lookup_evidence':lookups,
            'readiness':ready, 'stop_restart_crash': {'restart':fields.get('Restart',['none'])[0], 'restart_delay_sec':fields.get('RestartSec',[]), 'limits':{k:v for k,v in fields.items() if k.startswith('StartLimit')},'behavior':'Legacy TERM, 3s grace, KILL, 2s confirmation; PID-based resident observation; no adoption after manager death' if unit else 'No userspace lifecycle supervisor; kernel reaper cleans exited processes; Init never retries'},
            'must_run_before_sunlightd':before,'privileged_kernel_initialization':privileged,
            'policy':'OneShot' if name=='sunlight-swapd' or fields.get('Type')==['oneshot'] else 'Resident',
            'conditional_probe_starts':conditional_probes,'duplicate_startup_paths':conflicts,'sunlightd_unit':unit, 'classification':classification,'recommendation':reason,'migration_risk':risk})
    # Explicit session components and exceptional boot/test paths are separate
    # from compiled system services and from the complete executable catalog.
    special = [
        {'name':'sshl','launcher':'TTY / authenticated shell child','supervisor':'TTY tab/session tracking','launch_site':'services/tty_server/src/main.rs::spawn_tab / spawn_tab_from_active_shell','policy':'SessionBound','classification':'SessionManaged','reason':'Authenticated login/session grants; retain legitimate shell children'},
        {'name':'sunlight-vortex-shell','launcher':'SessionD','supervisor':'SessionD generation-qualified shell tracking and bounded restart','launch_site':'services/sunlight-sessiond/src/main.rs::spawn_shell / maybe_restart_shell','policy':'SessionBound','classification':'SessionManaged','reason':'Session authority and readiness handoff; not a system unit'},
        {'name':'configured startup applications (Welcome, Wise Owl and profile entries)','launcher':'SessionD','supervisor':'Profile completion tracking; no general service restart','launch_site':'services/sunlight-sessiond/src/config_ops.rs::spawn_optional; main.rs::try_launch_deferred_optional','policy':'SessionBound','classification':'SessionManaged','reason':'Per-user configuration and delayed desktop startup; inspect profile.rs and config_ops.rs'},
        {'name':'sunlight-runner / sunlight-clipman / emoji-picker','launcher':'Display on user action','supervisor':'Application-specific singleton PID tracking','launch_site':'services/sunlight-display/src/main.rs launch helpers','policy':'SessionBound','classification':'SessionManaged','reason':'User-invoked applications, not automatic system services'},
        {'name':'schedbench','launcher':'Kernel (scheduler_guest_bench feature only)','supervisor':None,'launch_site':'kernel/src/main.rs::boot Process::new(5)','policy':'OneShot','classification':'KeepExistingWithReason','reason':'Feature-gated benchmark, excluded from default bootstrap count'},
        {'name':'D1/D2/D3 fixtures and drivers','launcher':'SunlightD (demand/idle/dependency-test feature only)','supervisor':'SunlightD demand generations and cooperative reaping','launch_site':'services/sunlightd/src/demand_runtime.rs::add_test_units','policy':'OnDemand and resident test drivers','classification':'SunlightDOnDemand','reason':'Synthetic regression fixtures only; defaults unchanged; full definitions in source-call index'},
    ]
    fixtures = [
        ('d1-driver', 'Resident', [], 'demand-test'),
        ('d1-fixture', 'OnDemand', [], 'demand-test'),
        ('d1-timeout', 'OnDemand', [], 'demand-test'),
        ('d1-crash', 'OnDemand', [], 'demand-test'),
        ('d1-missing', 'OnDemand', [], 'demand-test'),
        ('d1-dependent', 'OnDemand', ['d1-fixture'], 'demand-test'),
        ('d2-driver', 'Resident', [], 'idle-test'),
        ('d2-fixture', 'OnDemand', [], 'idle-test'),
        ('d2-dependent', 'OnDemand', ['d2-fixture'], 'idle-test'),
        ('d3-leaf', 'OnDemand', ['d2-fixture'], 'dependency-test'),
        ('d3-parent', 'OnDemand', ['d3-leaf', 'd2-fixture'], 'dependency-test'),
        ('d1-denied', 'OneShot', [], 'driver child'),
        ('d1-activate-only', 'OneShot', [], 'driver child'),
        ('d3-client', 'OneShot', [], 'driver child'),
    ]
    for name, policy, dependencies, feature in fixtures:
        definition = {'file':'services/sunlightd/src/demand_runtime.rs','function':'add_test_units'}
        services.append({'name':name, 'executable':'/sbin/'+name,
            'source':next((b['source'] for b in binaries if b['name']==name), None),
            'launch_site': definition if feature != 'driver child' else {'file':'services/sunlight-startup-fixture/src','function':'driver test spawn helper'},
            'launcher':'SunlightD' if feature != 'driver child' else 'Test driver',
            'supervisor':'SunlightD D1/D2/D3' if policy=='OnDemand' else 'Test driver assertions / manager legacy resident path',
            'registered_names': [name] if name not in ('d1-missing','d1-crash','d1-timeout') else [],
            'endpoint':'Runtime-created; absent for intentional failed startup',
            'registration_evidence':[c for c in calls if c['file'].startswith('services/sunlight-startup-fixture/') and c['operation']=='nameserver_register'],
            'capability_requirements':'service-notify + logging for demand fixtures; driver service-lifecycle/demand-fixture/spawn-user/time-sync/logging; child scoped test profiles in driver source',
            'dependencies':dependencies, 'runtime_lookup_evidence':[],
            'readiness':'D1 HELLO/READY with kernel process generation and endpoint ownership' if policy=='OnDemand' else 'Test assertions / legacy spawn success',
            'stop_restart_crash':'D1 bounded startup cleanup; D2/D3 cooperative drain and generation-qualified reaping where opted in; explicit retry after failure',
            'must_run_before_sunlightd':False, 'privileged_kernel_initialization':False,
            'policy':policy, 'duplicate_startup_paths':[], 'sunlightd_unit':definition if feature != 'driver child' else None,
            'classification':'SunlightDOnDemand' if policy=='OnDemand' else 'KeepExistingWithReason',
            'recommendation':'Retain synthetic regression behavior; not a production migration', 'migration_risk':'Test-only; feature='+feature, 'conditional':feature})
    counts = {'kernel_direct_default':3,'init_direct_default':len(init_paths),'sunlightd_default_definitions':sum(not u['conditional'] for u in units),'sunlightd_default_enabled':sum(u['default_enabled'] and not u['conditional'] for u in units)}
    return {'schema_version':1,'scope':'Source audit; dynamic runtime measurements are separate. Lists are configuration, not observed process counts.',
            'counts':counts, 'init_boot_order':init_paths, 'compiled_units':units, 'services':services,'session_and_conditional_processes':special,'embedded_executables':embedded,
            'cargo_executable_catalog':binaries,'source_call_index':calls,
            'source_hashes':{p:hashlib.sha256(read(p).encode()).hexdigest() for p in sources},
            'initd':'No executable or implementation found; historical documentation uses init/initd terminology',
            'limitations':['Lexical census includes tests/comments; call evidence requires review, and is not a Rust call graph.','Optional Cargo targets and embedded ELF artifacts are not proof of an automatic process launch.','Unresolved registration constants and runtime-selected app paths are retained in source_call_index; no numeric endpoint is invented.']}

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--output',type=Path,default=ROOT/'target/d35-evidence/current-inventory.json')
    parser.add_argument('--check-owners',action='store_true')
    parser.add_argument('--snapshot',type=Path)
    parser.add_argument('--graph',type=Path)
    args=parser.parse_args()
    data=build()
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(data,indent=2)+'\n')
    if args.snapshot:
        snapshot={k:v for k,v in data.items() if k not in ('source_hashes','source_call_index')}
        args.snapshot.write_text(json.dumps(snapshot,indent=2)+'\n')
    if args.graph:
        edges=[('init','sunlightd','nameserver / trusted bootstrap'),
               ('vfs_server','sunlightd','credentials / enabled-state I/O'),
               ('Kernel','vfs_server','read-only FAT boot-share mapping'),
               ('Kernel','tty_server','framebuffer / trusted TTY marker')]
        for component in data['session_and_conditional_processes']:
            edges.append((component['launcher'],component['name'],'session / conditional launch'))
        for service in data['services']:
            edges.append((service['launcher'],service['name'],'launches'))
            deps=service['dependencies']
            if isinstance(deps,dict):
                for kind in ('requires','after','wants_advisory'):
                    edges.extend((dep.removesuffix('.service'),service['name'],kind) for dep in deps[kind])
            elif isinstance(deps,list):
                edges.extend((dep,service['name'],'requires') for dep in deps)
        args.graph.write_text('digraph ownership {\n  rankdir=LR;\n'+''.join(
            '  '+json.dumps(a)+' -> '+json.dumps(b)+' [label='+json.dumps(kind)+'];\n'
            for a,b,kind in edges)+'}\n')
    if args.check_owners:
        duplicates=[s['name'] for s in data['services'] if s['duplicate_startup_paths']]
        init=set(data['init_boot_order'])
        managed_list=[u['executable'] for u in data['compiled_units']]
        managed=set(managed_list)
        assert len(managed_list) == len(managed), 'Duplicate compiled service executable'
        init_list=data['init_boot_order']
        assert len(init_list) == len(set(init_list)), 'Duplicate Init list entry'
        assert not init & managed, f'Init/manager overlap: {init & managed}'
        assert not duplicates, f'Independent fallback service spawns: {duplicates}'
    print(json.dumps({'output':str(args.output),'counts':data['counts'],'services':len(data['services']), 'embedded_artifacts':len(data['embedded_executables']),'cargo_targets':len(data['cargo_executable_catalog'])}))

if __name__=='__main__':
    main()
