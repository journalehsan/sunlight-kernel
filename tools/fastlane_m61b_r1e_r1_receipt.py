"""Strict positive hybrid receipt/physical-retirement audit; never milestone acceptance."""
import json,re,sys
from pathlib import Path

def assess(raw):
    errors=[]
    def require(ok,why):
        if not ok: errors.append(why)
    def rows(prefix):
        out=[]
        for line in raw.splitlines():
            if line.startswith(prefix+' '):
                pairs=re.findall(r'(\w+)=(\S+)',line);r=dict(pairs)
                require(len(r)==len(pairs),'duplicate field '+prefix)
                out.append(r)
        return out
    def one(items,key,value):
        found=[r for r in items if r.get(key)==value]
        require(len(found)==1,'exactly one '+key+'='+value)
        return found[0] if found else {}
    try:
        users=rows('[FL-M61E]'); auth=one(rows('[FL-HYBRID-K]'),'event','authorized')
        attachments=[r for r in rows('[FL-AUTH]') if r.get('op')=='3' and r.get('result')=='0']
        require(len(attachments)==2,'two one-use attachments')
        require({r['cpu'] for r in attachments}=={'0','1'},'native two-CPU attachment witnesses')
        for pid,inc,vm in [('caller','ci','cv'),('service','si','sv')]:
            a=one(attachments,'pid',auth[pid])
            require(a.get('inc')==auth[inc] and a.get('vm')==auth[vm] and a.get('generation')==auth['channel'],'attachment identity')
        generation=auth['channel']; require(auth['operation']=='65552' and auth['request']=='FastLaneRequired' and auth['reply']=='KernelIPC','directional authority')
        require(auth['caller']!=auth['service'] and auth['ci']==auth['call_incarnation'],'reply caller identity')
        for key in ['channel','policy','pg','caller','ci','cv','service','si','sv','call_generation']:
            require(int(auth[key])>0,'nonzero '+key)
        begin=one(rows('[FL-M61B-K]'),'event','begin')
        for k,v in [('g',generation),('caller',auth['caller']),('ci',auth['ci']),('cv',auth['cv']),('service',auth['service']),('si',auth['si']),('sv',auth['sv']),('policy_id',auth['policy']),('policy_generation',auth['pg'])]:
            require(begin.get(k)==v,'contract join '+k)
        for event in ['directional','request-published','reply-created','private-copy','actor-executed','duplicate-reply','response-published','acknowledged','complete']:
            one(users,'case',event)
        require(one(users,'case','request-published').get('channel')==generation,'request join')
        require(one(users,'case','private-copy').get('channel')==generation,'private copy join')
        require(one(users,'case','reply-created').get('channel')==generation,'reply authority join')
        accepted=one(users,'case','acknowledged')
        require(accepted.get('request')=='0123456789abcdef' and accepted.get('response')=='fedcba9876543210' and accepted.get('terminal')=='success','exact operation acceptance')
        frames=rows('[FL-FRAME]');maps=rows('[FL-MAP]');tlbs=rows('[FL-TLB]')
        require(len([r for r in frames if r['event']=='backing'])==3,'three backing frames')
        require(len([r for r in frames if r['event']=='free'])==3,'three physical releases')
        require(len([r for r in maps if r['event']=='map'])==6,'six restricted maps')
        require(len([r for r in maps if r['event']=='complete'])==6,'six completed unmaps')
        physical=[];remote=0
        for region in range(3):
            region=str(region)
            b=one([r for r in frames if r['event']=='backing'],'region',region)
            f=one([r for r in frames if r['event']=='free'],'region',region)
            require(b['frame']==f['frame'] and b['generation']==f['generation']==generation,'frame lifetime join')
            for role in range(2):
                m=one([r for r in maps if r['event']=='map' and r['region']==region],'role',str(role))
                require(m['generation']==generation and m['frame']==b['frame'] and m['nx']=='1' and m['write']==str(int((region=='1')==(role==1))),'restricted map rights')
                for key,val in [('pid',auth['caller' if role==0 else 'service']),('inc',auth['ci' if role==0 else 'si']),('vm',auth['cv' if role==0 else 'sv'])]:
                    require(m[key]==val,'mapped identity '+key)
                scoped=[r for r in maps if r['generation']==generation and r['region']==region and r['vm']==m['vm'] and r['va']==m['va']]
                req=one(scoped,'event','request');done=one(scoped,'event','complete')
                t=[r for r in tlbs if r['generation']==generation and r['region']==region and r['vm']==m['vm'] and r['va']==m['va']]
                tr=one(t,'event','request');tc=one(t,'event','complete');mask=int(tr['targets']);remote+=mask.bit_count()
                require(tr['request']==tc['request'] and tr['targets']==tc['targets'],'TLB request join')
                for cpu in range(64):
                    if mask & (1<<cpu):
                        ack=one([r for r in t if r['event']=='ack'],'cpu',str(cpu))
                        require(ack.get('observed')==tr['request'],'remote ACK join')
                require(int(b['seq'])<int(m['seq'])<int(req['seq'])<int(tr['seq'])<int(tc['seq'])<int(done['seq'])<int(f['seq']),'unmap/TLB/free ordering')
            physical.append({'region':region,'frame':b['frame'],'backing_seq':b['seq'],'release_seq':f['seq']})
        require(len([r for r in tlbs if r['event']=='request'])==6 and len([r for r in tlbs if r['event']=='complete'])==6,'six TLB fences')
        negatives=[r for r in users if r.get('case')=='negative']
        for r in negatives:
            require(r.get('terminal')=='rejected' and r.get('count')=='1','one negative terminal')
        require(len({r.get('control') for r in negatives})==len(negatives),'duplicate negative terminal')
        census=one(rows('[FL-CENSUS]'),'checkpoint','690')
        require(all(census.get(k)=='0' for k in ['objects','views','frames']),'scoped M2 inventory')
        require(not any(m in raw for m in ['checkpoint=FAIL','PANIC','panicked at']),'guest assertion')
        return {'status':'FAIL' if errors else 'PASS','scope':'positive hybrid operation and physical retirement only','milestone':'PARTIAL','errors':errors,'physical':physical,'remote_tlb_targets':remote,'remote_ack_proof':'OBSERVED' if remote else 'NOT_EXERCISED','native_negative_controls':[r['control'] for r in users if r.get('case')=='negative']}
    except (KeyError,ValueError,TypeError) as e:
        return {'status':'FAIL','errors':errors+[str(e)],'milestone':'PARTIAL'}
if __name__=='__main__':
    result=assess(Path(sys.argv[1]).read_text(errors='replace'));print(json.dumps(result,indent=2));sys.exit(result['status']!='PASS')
