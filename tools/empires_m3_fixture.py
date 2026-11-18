#!/usr/bin/env python3
"""Build affected native services before the existing init-only UI fixture.
Kernel embedding accepts existing ELF artifacts without source freshness checks.
Do not run concurrently with production/other image builds in this workspace.
"""
from pathlib import Path
import argparse
import os
import subprocess
import sys
ROOT=Path(__file__).resolve().parents[1]
FLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',default='target/empires-m3/validated-native');a=p.parse_args()
    subprocess.run(['cargo','build','-p','sunlight-empires','-p','sunlight-silicon-echoes','-p','sunlight-vfs-server','--features','sunlight-empires/native','--release'],cwd=ROOT,env=dict(os.environ,RUSTFLAGS=FLAGS),check=True)
    subprocess.run([sys.executable,'tools/empires_m1_fixture.py','--output',a.output],cwd=ROOT,check=True)
if __name__=='__main__':main()
