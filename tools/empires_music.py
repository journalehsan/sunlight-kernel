#!/usr/bin/env python3
"""Rebuild canonical PCM excerpts from retained official-archive MP3s.
Requires ffmpeg. Nothing is downloaded; inputs are hash-locked in attribution.json.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import wave
ROOT = Path(__file__).resolve().parents[1]
BUNDLE = ROOT / 'SunlightEmpires.sunapp'

def main():
    inventory = json.loads((BUNDLE/'Licenses/attribution.json').read_text())
    for track in inventory['tracks']:
        source = BUNDLE / track['original']
        assert hashlib.sha256(source.read_bytes()).hexdigest() == track['original_sha256'], source
        raw = subprocess.run(['ffmpeg','-v','error','-i',str(source),'-t','30','-af',
            'afade=t=in:st=0:d=0.02,afade=t=out:st=29:d=1',
            '-ar','48000','-ac','2','-f','s16le','-acodec','pcm_s16le','-'],check=True,stdout=subprocess.PIPE).stdout
        assert len(raw) == 30*192000
        destination = BUNDLE / track['resource']
        with wave.open(str(destination),'wb') as out:
            out.setnchannels(2); out.setsampwidth(2); out.setframerate(48000); out.writeframes(raw)
        digest = hashlib.sha256(destination.read_bytes()).hexdigest()
        assert digest == track['resource_sha256'], (destination, digest, 'converter version drift; review before updating lock')
        print(destination.relative_to(ROOT), digest)
if __name__ == '__main__': main()
