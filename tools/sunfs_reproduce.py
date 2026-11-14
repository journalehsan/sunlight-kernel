#!/usr/bin/env python3
"""Reproduce one recorded bounded fuzz input into a NEW disposable file.

Corpus identity is (seed,index,kind) plus source/golden SHA256 in the evidence.
Uses the recorded deterministic bootstrap image, never calls a trusted recovery
selector or writes to an existing image. Checker remains a separate read-only CLI.
"""
import argparse,gzip,json
from pathlib import Path
from sunfs_golden import crc64
ROOT=Path(__file__).resolve().parents[1]
MASK=(1<<64)-1

def step(r):
    r^=(r<<13)&MASK;r^=r>>7;r^=(r<<17)&MASK
    return r

def reproduce(seed,index,kind):
    assert 0<=index<512 and 0<=seed<=MASK
    r=(seed+0x9e3779b97f4a7c15)&MASK
    base=ROOT/'docs/audit/sunfs/sfs-1/raw/release/bootstrap.img.gz'
    if kind=='image-mutation':
        for _ in range(index+1):r=step(r)
        block=r%5 if index%2==0 else 256+r%8
        offset=(r>>17)%4096
        data=bytearray(gzip.decompress(base.read_bytes()))
        data[block*4096+offset]^=((r>>32)&255)|1
        if index%3==0:
            page=bytearray(data[block*4096:(block+1)*4096]);page[64:72]=bytes(8);page[64:72]=crc64(page).to_bytes(8,'little');data[block*4096:(block+1)*4096]=page
        identity=crc64(data[block*4096:(block+1)*4096])
    else:
        for _ in range(512+index*4096):r=step(r)
        data=bytearray(4096)
        for i in range(4096):r=step(r);data[i]=r&255
        identity=crc64(data)
    return data,f'{identity:016x}'

def main():
    p=argparse.ArgumentParser();p.add_argument('--seed',type=lambda x:int(x,0),required=True);p.add_argument('--index',type=int,required=True);p.add_argument('--kind',choices=['image-mutation','arbitrary-page'],required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
    data,identity=reproduce(a.seed,a.index,a.kind)
    corpus=ROOT/'docs/audit/sunfs/sfs-1/raw/release/fuzz-corpus.jsonl'
    rows=[json.loads(line) for line in corpus.read_text().splitlines()]
    row=next(r for r in rows if r['seed']==a.seed and r['index']==a.index and r['kind']==a.kind)
    assert identity==row.get('mutated_page_crc64',row.get('input_crc64')),(identity,row)
    with a.output.open('xb') as f:f.write(data)
    print(json.dumps({'output':str(a.output),'identity_crc64':identity,'recorded_classification':row['classification']}))
if __name__=='__main__':main()
