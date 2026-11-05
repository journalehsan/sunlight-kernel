"""Fail-closed sequential caller exec pilot receipts; never concurrency proof."""
from fastlane_m61b_r2_validate import rows

def assess(raw):
    errors=[]
    def one(prefix,event):
        found=[r for r in rows(raw,prefix) if r.get('event')==event]
        if len(found)!=1:
            errors.append(f'{event}: expected one receipt, got {len(found)}')
            return {}
        return found[0]
    call=one('[FL-P1] ','raw-call-enter')
    held=one('[FL-P1] ','held')
    authorized=one('[FL-HYBRID-K] ','authorized')
    enter=one('[FL-P1-K] ','exec-enter')
    installed=one('[FL-P1-K] ','exec-installed')
    replacement=one('[FL-P1] ','replacement-entry')
    late=one('[FL-P1] ','late-reply')
    complete=one('[FL-P1] ','complete')
    terminals=[r for r in rows(raw,'[FL-HYBRID-K] ') if r.get('event') in ('terminal','retired')]
    if len(terminals)!=1 or terminals[0].get('outcome')!='CallerRetired':
        errors.append('missing unique CallerRetired decision')
    if enter.get('pending')!='1' or enter.get('owns_vm')!='true' or enter.get('path')!='/bin/session-racer':
        errors.append('exec did not enter with owned VM and outstanding call')
    if not enter.get('old_vm') or enter.get('old_vm')==installed.get('new_vm'):
        errors.append('distinct replacement VM not proven')
    if len({r.get('caller') for r in (call,authorized,enter,installed,replacement,complete)})!=1:
        errors.append('caller identity mismatch')
    if enter.get('ci')!=installed.get('ci') or enter.get('ci')!=authorized.get('ci') or enter.get('old_vm')!=authorized.get('cv'):
        errors.append('incarnation/VM mismatch')
    if len({r.get('channel') for r in (call,held,authorized,replacement,late,complete,*terminals)})!=1:
        errors.append('channel correlation mismatch')
    if replacement.get('old_vm_adoption')!='rejected' or late.get('accepted')!='0' or late.get('duplicate')!='rejected':
        errors.append('negative control absent')
    inventory=rows(raw,'[FL-R2-INVENTORY] ')
    if len(inventory)!=1 or inventory[0].get('total')!='0' or inventory[0].get('channel')!=call.get('channel'):
        errors.append('zero scoped inventory absent')
    # Physical lifetime assessment is joined independently to the existing
    # frame/TLB observer by the pilot audit, not inferred from inventory zero.
    ordered=['raw-call-enter','held','exec-enter','exec-installed','replacement-entry','late-reply','complete']
    positions=[raw.find('event='+event+' ') for event in ordered]
    if min(positions)<0 or positions!=sorted(positions): errors.append('sequential order absent')
    if any(marker in raw for marker in ('PANIC','panicked at','[FAULT]')): errors.append('guest failure')
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'scope':'sequential caller-owned exec only',
            'call':call,'authorized':authorized,'enter':enter,'installed':installed,'replacement':replacement,
            'late':late,'terminals':terminals,'inventory':inventory,'physical_retirement':'requires independent audit'}
